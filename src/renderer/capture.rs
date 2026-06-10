// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::cef::cdp::Cdp;
use crate::host::render::RenderRef;
use cef::*;
use std::sync::{Arc, Mutex};

pub type BrowserHandle = Arc<Mutex<Option<Browser>>>;
pub type FrameHandle = Arc<Mutex<Option<Frame>>>;

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

pub fn step_frame(render: &RenderRef) {
    let (cdp, browser, iframe, budget_ms, ms) = {
        let r = render.lock().expect("render poisoned");
        if r.done { return; }
        let ms = (((r.capture.next_frame + 1) as f64) * r.frame_ms).floor() as u32;
        (r.cdp.clone(), r.browser.clone(), r.iframe.clone(), r.frame_ms as u32, ms)
    };

    let Some(host) = browser.lock().expect("browser poisoned").as_ref().and_then(|b| b.host()) else {
        return;
    };

    {
        let mut r = render.lock().expect("render poisoned");
        r.capture.requested_ms = ms;
        r.capture.budget_done = false;
        r.capture.stuck_invalidates = 0;
    }

    draw_stego(&cdp, &host, ms);
    if let Some(f) = iframe.lock().expect("iframe poisoned").as_ref() {
        tick_iframe(f, ms);
    }
    advance_virtual_time(&cdp, &host, budget_ms);
}

wrap_task! {
    pub struct StepTask { pub render: RenderRef }
    impl Task {
        fn execute(&self) { step_frame(&self.render); }
    }
}

pub fn schedule_step(render: &RenderRef) {
    let mut task = StepTask::new(render.clone());
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

pub fn install_budget_listener(render: &RenderRef) {
    let (cdp, weak) = {
        let r = render.lock().expect("render poisoned");
        (r.cdp.clone(), Arc::downgrade(render))
    };
    // Weak so a finished/removed render's listener is a no-op, never resurrects it.
    cdp.on_event("Emulation.virtualTimeBudgetExpired", move |_| {
        let Some(render) = weak.upgrade() else { return };
        let browser = {
            let mut r = render.lock().expect("render poisoned");
            r.capture.budget_done = true;
            r.browser.clone()
        };
        schedule_invalidate(browser);
    });
}
