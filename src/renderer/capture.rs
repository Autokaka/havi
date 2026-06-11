// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::cef::cdp::Cdp;
use cef::*;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

pub type BrowserHandle = Arc<Mutex<Option<Browser>>>;
pub type FrameHandle = Arc<Mutex<Option<Frame>>>;

pub struct State {
    pub next_frame: u32,
    pub requested_ms: u32,
    pub budget_done: bool,
    pub stuck_invalidates: u32,
    pub tx: Option<SyncSender<Vec<u8>>>,
    pub done: bool,
    pub browser: BrowserHandle,
    pub iframe: FrameHandle,
    pub cdp: Cdp,
    pub width: i32,
    pub height: i32,
    pub total_frames: u32,
    pub frame_ms: f64,
}

pub type Shared = Arc<Mutex<State>>;

// Wall-clock guard per frame. If a frame produces no matching paint in this
// window (e.g. virtualTimeBudgetExpired never fires), abort instead of hanging.
const FRAME_TIMEOUT_MS: i64 = 5000;

pub fn pause_virtual_time(cdp: &Cdp, host: &BrowserHost) {
    cdp.send(host, "Emulation.setVirtualTimePolicy", r#"{"policy":"pause"}"#);
}

fn advance_virtual_time(cdp: &Cdp, host: &BrowserHost, budget_ms: u32) {
    let params = format!(
        r#"{{"policy":"advance","budget":{budget_ms},"maxVirtualTimeTaskStarvationCount":1}}"#
    );
    cdp.send(host, "Emulation.setVirtualTimePolicy", &params);
}

fn draw_stego(cdp: &Cdp, host: &BrowserHost, ms: u32) {
    let params = format!(r#"{{"expression":"window.__havi_step({ms})","returnByValue":true}}"#);
    cdp.send(host, "Runtime.evaluate", &params);
}

fn tick_iframe(frame: &Frame, ms: u32) {
    let code = format!(
        "(function(){{if(window.__havi_tick)window.__havi_tick.process({ms});if(window.__havi_advance_videos)window.__havi_advance_videos({ms});}})()"
    );
    frame.execute_java_script(Some(&CefString::from(code.as_str())), None, 0);
}

pub fn step_frame(state: &Shared) {
    let (cdp, browser, iframe, budget_ms, ms, frame) = {
        let cap = state.lock().expect("state poisoned");
        if cap.done { return; }
        let ms = (((cap.next_frame + 1) as f64) * cap.frame_ms).floor() as u32;
        (cap.cdp.clone(), cap.browser.clone(), cap.iframe.clone(), cap.frame_ms as u32, ms, cap.next_frame)
    };

    let Some(host) = browser.lock().expect("browser poisoned").as_ref().and_then(|b| b.host()) else {
        return;
    };

    {
        let mut cap = state.lock().expect("state poisoned");
        cap.requested_ms = ms;
        cap.budget_done = false;
        cap.stuck_invalidates = 0;
    }

    draw_stego(&cdp, &host, ms);
    if let Some(f) = iframe.lock().expect("iframe poisoned").as_ref() {
        tick_iframe(f, ms);
    }
    advance_virtual_time(&cdp, &host, budget_ms);
    schedule_frame_watchdog(state, frame);
}

wrap_task! {
    pub struct FrameWatchdog { pub state: Shared, pub frame: u32 }
    impl Task {
        fn execute(&self) {
            let mut cap = self.state.lock().expect("state poisoned");
            if cap.done || cap.next_frame > self.frame { return; }
            let ms = cap.requested_ms;
            cap.done = true;
            cap.tx = None;
            drop(cap);
            crate::ipc::error(&format!("frame {} timeout: no paint in {FRAME_TIMEOUT_MS}ms (ms={ms})", self.frame));
            quit_message_loop();
        }
    }
}

fn schedule_frame_watchdog(state: &Shared, frame: u32) {
    let mut task = FrameWatchdog::new(state.clone(), frame);
    post_delayed_task(ThreadId::UI, Some(&mut task), FRAME_TIMEOUT_MS);
}

wrap_task! {
    pub struct StepTask { pub state: Shared }
    impl Task {
        fn execute(&self) { step_frame(&self.state); }
    }
}

pub fn schedule_step(state: &Shared) {
    let mut task = StepTask::new(state.clone());
    post_task(ThreadId::UI, Some(&mut task));
}

wrap_task! {
    pub struct InvalidateTask { pub browser: BrowserHandle }
    impl Task {
        fn execute(&self) {
            let host = self.browser.lock().expect("browser poisoned").as_ref().and_then(|b| b.host());
            if let Some(h) = host { h.invalidate(PaintElementType::VIEW); }
        }
    }
}

pub fn schedule_invalidate(browser: BrowserHandle) {
    let mut task = InvalidateTask::new(browser);
    post_task(ThreadId::UI, Some(&mut task));
}

pub fn install_budget_listener(state: Shared) {
    let cdp = state.lock().expect("state poisoned").cdp.clone();
    cdp.on_event("Emulation.virtualTimeBudgetExpired", move |_| {
        let browser = {
            let mut s = state.lock().expect("state poisoned");
            s.budget_done = true;
            s.browser.clone()
        };
        schedule_invalidate(browser);
    });
}
