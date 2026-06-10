// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

use crate::cef::cdp::Cdp;
use crate::ipc::RenderId;
use crate::renderer::capture::{BrowserHandle, FrameHandle};
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
    pub encoder_pid: Option<u32>,
    pub tx: Option<SyncSender<Vec<u8>>>,
    pub done: bool,
    pub started_at: Instant,
}

pub type RenderRef = Arc<Mutex<Render>>;

#[derive(Default)]
pub struct Host {
    renders: Mutex<HashMap<RenderId, RenderRef>>,
    by_browser: Mutex<HashMap<i32, RenderId>>,
}

impl Host {
    pub fn new() -> Self { Self::default() }

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

    pub fn remove(&self, id: RenderId) -> Option<RenderRef> {
        self.by_browser.lock().expect("by_browser poisoned").retain(|_, v| *v != id);
        self.renders.lock().expect("renders poisoned").remove(&id)
    }

    pub fn active_count(&self) -> usize {
        self.renders.lock().expect("renders poisoned").len()
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
            encoder_pid: None, tx: None, done: false, started_at: Instant::now(),
        }))
    }

    #[test]
    fn route_by_browser_id() {
        let host = Host::new();
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
        let host = Host::new();
        host.insert(fake_render(1));
        host.bind_browser(101, 1);
        assert_eq!(host.active_count(), 1);
        host.remove(1);
        assert_eq!(host.active_count(), 0);
        assert!(host.by_browser(101).is_none());
    }
}
