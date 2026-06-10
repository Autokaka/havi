// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

use crate::cef::cdp::Cdp;
use crate::host::ipc::emit_evt;
use crate::ipc::{Evt, RenderId};
use crate::renderer::capture::{BrowserHandle, FrameHandle};
use crate::video::encoder::EncoderHandle;
use cef::{quit_message_loop, ImplBrowser, ImplBrowserHost, Registration};
use std::collections::HashMap;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct CaptureState {
    pub next_frame: u32,
    pub requested_ms: u32,
    pub budget_done: bool,
    pub stuck_invalidates: u32,
}

pub struct Render {
    pub id: RenderId,
    pub width: i32,
    pub height: i32,
    pub total_frames: u32,
    pub frame_ms: f64,
    pub out: String,
    pub tolerant: bool,
    pub phase: u8,
    pub browser: BrowserHandle,
    pub iframe: FrameHandle,
    pub cdp: Cdp,
    pub capture: CaptureState,
    pub encoder: Option<EncoderHandle>,
    pub tx: Option<SyncSender<Vec<u8>>>,
    pub done: bool,
    pub errored: bool,
    pub devtools: Option<Registration>,
    pub started_at: Instant,
}

pub type RenderRef = Arc<Mutex<Render>>;

pub type StartFn = Box<dyn Fn(&Arc<Host>, RenderId, crate::api::RenderOpts) + Send + Sync>;

pub struct Host {
    renders: Mutex<HashMap<RenderId, RenderRef>>,
    by_browser: Mutex<HashMap<i32, RenderId>>,
    queue: Mutex<std::collections::VecDeque<(RenderId, crate::api::RenderOpts)>>,
    max_parallel: usize,
    start_fn: Mutex<Option<StartFn>>,
    single_shot: Mutex<Option<RenderId>>,
    creating: Mutex<Option<RenderRef>>,
}

impl Host {
    pub fn new(max_parallel: usize) -> Arc<Self> {
        Arc::new(Self {
            renders: Mutex::new(HashMap::new()),
            by_browser: Mutex::new(HashMap::new()),
            queue: Mutex::new(std::collections::VecDeque::new()),
            max_parallel: max_parallel.max(1),
            start_fn: Mutex::new(None),
            single_shot: Mutex::new(None),
            creating: Mutex::new(None),
        })
    }

    // CEF calls view_rect during browser creation, before browser_id is known
    // to bind. Serialized on the UI thread, so one pending render at a time.
    pub fn set_creating(&self, render: RenderRef) {
        *self.creating.lock().expect("creating poisoned") = Some(render);
    }

    pub fn clear_creating(&self) {
        *self.creating.lock().expect("creating poisoned") = None;
    }

    pub fn creating(&self) -> Option<RenderRef> {
        self.creating.lock().expect("creating poisoned").clone()
    }

    pub fn set_start_fn(&self, f: StartFn) {
        *self.start_fn.lock().expect("start_fn poisoned") = Some(f);
    }

    pub fn set_single_shot(&self, id: RenderId) {
        *self.single_shot.lock().expect("single_shot poisoned") = Some(id);
    }

    fn maybe_quit_single_shot(&self, finished: RenderId) {
        let target = *self.single_shot.lock().expect("single_shot poisoned");
        if target == Some(finished)
            && self.active_count() == 0
            && self.queue.lock().expect("queue poisoned").is_empty()
        {
            quit_message_loop();
        }
    }

    pub fn dom_ready_advance(self: &Arc<Self>) {
        let target = {
            let renders = self.renders.lock().expect("renders poisoned");
            renders.values()
                .find(|r| { let g = r.lock().expect("render poisoned"); g.tolerant && g.phase < 2 })
                .cloned()
        };
        if let Some(render) = target {
            crate::renderer::load::advance_phase(self, &render, None);
        }
    }

    pub fn insert(&self, render: RenderRef) {
        let id = render.lock().expect("render poisoned").id;
        self.renders.lock().expect("renders poisoned").insert(id, render);
    }

    pub fn bind_browser(&self, browser_id: i32, render_id: RenderId) {
        self.by_browser.lock().expect("by_browser poisoned").insert(browser_id, render_id);
    }

    pub fn by_id(&self, id: RenderId) -> Option<RenderRef> {
        self.renders.lock().expect("renders poisoned").get(&id).cloned()
    }

    pub fn by_browser(&self, browser_id: i32) -> Option<RenderRef> {
        let id = *self.by_browser.lock().expect("by_browser poisoned").get(&browser_id)?;
        self.by_id(id)
    }

    pub fn active_count(&self) -> usize {
        self.renders.lock().expect("renders poisoned").len()
    }

    pub fn submit(self: &Arc<Self>, id: RenderId, opts: crate::api::RenderOpts) {
        let at_cap = self.active_count() >= self.max_parallel;
        if at_cap {
            self.queue.lock().expect("queue poisoned").push_back((id, opts));
            return;
        }
        self.spawn_render(id, opts);
    }

    fn spawn_render(self: &Arc<Self>, id: RenderId, opts: crate::api::RenderOpts) {
        let f = self.start_fn.lock().expect("start_fn poisoned");
        if let Some(start) = f.as_ref() {
            start(self, id, opts);
        }
    }

    fn drain_queue(self: &Arc<Self>) {
        let next = self.queue.lock().expect("queue poisoned").pop_front();
        if let Some((id, opts)) = next {
            self.spawn_render(id, opts);
        }
    }

    pub fn finish(self: &Arc<Self>, id: RenderId) {
        let Some(render) = self.remove(id) else { return };
        let (encoder, out, frames, started, errored) = {
            let mut r = render.lock().expect("render poisoned");
            r.done = true;
            r.tx = None;
            (r.encoder.take(), r.out.clone(), r.capture.next_frame, r.started_at, r.errored)
        };
        if let Some(b) = render.lock().expect("render poisoned").browser.lock().expect("browser").take() {
            if let Some(h) = b.host() { h.close_browser(1); }
        }
        let mut ok = true;
        if let Some(enc) = encoder {
            ok = enc.finish().map(|s| s.success()).unwrap_or(false);
        }
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if !errored {
            if ok {
                emit_evt(&Evt::Done { id, out, frames, elapsed_ms });
            } else {
                emit_evt(&Evt::Error { id, message: "ffmpeg encoder failed".into() });
            }
        }
        self.drain_queue();
        self.maybe_quit_single_shot(id);
    }

    pub fn cancel(self: &Arc<Self>, id: RenderId) {
        let Some(render) = self.by_id(id) else { return };
        {
            let mut r = render.lock().expect("render poisoned");
            r.errored = true;
            r.done = true;
            r.tx = None;
        }
        emit_evt(&Evt::Error { id, message: "cancelled".into() });
        self.remove(id);
        if let Some(enc) = render.lock().expect("render poisoned").encoder.take() {
            let _ = enc.finish();
        }
        if let Some(b) = render.lock().expect("render poisoned").browser.lock().expect("browser").take() {
            if let Some(h) = b.host() { h.close_browser(1); }
        }
        self.drain_queue();
        self.maybe_quit_single_shot(id);
    }

    fn remove(&self, id: RenderId) -> Option<RenderRef> {
        self.by_browser.lock().expect("by_browser poisoned").retain(|_, v| *v != id);
        self.renders.lock().expect("renders poisoned").remove(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_render(id: RenderId) -> RenderRef {
        Arc::new(Mutex::new(Render {
            id, width: 16, height: 16, total_frames: 1, frame_ms: 33.0,
            out: "o.mp4".into(), tolerant: false, phase: 0,
            browser: Arc::new(Mutex::new(None)),
            iframe: Arc::new(Mutex::new(None)),
            cdp: Cdp::new(),
            capture: CaptureState { next_frame: 0, requested_ms: 0, budget_done: false, stuck_invalidates: 0 },
            encoder: None, tx: None, done: false, errored: false, devtools: None,
            started_at: Instant::now(),
        }))
    }

    #[test]
    fn route_by_browser_id() {
        let host = Host::new(4);
        host.insert(fake_render(1));
        host.insert(fake_render(2));
        host.bind_browser(101, 1);
        host.bind_browser(102, 2);
        assert_eq!(host.by_browser(101).unwrap().lock().unwrap().id, 1);
        assert_eq!(host.by_browser(102).unwrap().lock().unwrap().id, 2);
        assert!(host.by_browser(999).is_none());
    }

    #[test]
    fn remove_clears_browser_binding() {
        let host = Host::new(4);
        host.insert(fake_render(1));
        host.bind_browser(101, 1);
        assert_eq!(host.active_count(), 1);
        host.remove(1);
        assert_eq!(host.active_count(), 0);
        assert!(host.by_browser(101).is_none());
    }
}
