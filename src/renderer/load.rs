// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::ipc;
use crate::renderer::capture::{pause_virtual_time, schedule_step, Shared};
use cef::*;
use std::sync::{Arc, Mutex};

// Real-time grace between phase 0→1 and reload. Lets HTTP cache + initial
// ffmpeg decode warm up before render begins (pup-recorder pattern).
const WARMUP_MS: i64 = 2000;
// Tolerant total deadline. Slow/broken load → ship with what's there.
pub const LOAD_TIMEOUT_MS: i64 = 15000;

wrap_task! {
    pub struct ReloadTask { pub state: Shared }
    impl Task {
        fn execute(&self) {
            let browser = {
                let mut s = self.state.lock().expect("state poisoned");
                s.reload_fired = true;
                s.browser.clone()
            };
            let main = browser.lock().expect("browser poisoned").as_ref().and_then(|b| b.main_frame());
            if let Some(main) = main {
                let url = CefString::from(&main.url()).to_string();
                main.load_url(Some(&CefString::from(url.as_str())));
            }
        }
    }
}

wrap_task! {
    pub struct TolerantTimeoutTask {
        pub state: Shared,
        pub phase: Arc<Mutex<u8>>,
        pub tolerant: bool,
    }
    impl Task {
        fn execute(&self) {
            let phase_v = *self.phase.lock().expect("phase poisoned");
            if phase_v >= 2 { return; }
            if !self.tolerant {
                ipc::error(&format!("load timeout after {LOAD_TIMEOUT_MS}ms (pass --tolerant to proceed anyway)"));
                std::process::exit(1);
            }
            ipc::error(&format!("load timeout after {LOAD_TIMEOUT_MS}ms — proceeding tolerant"));
            // Force progression to render. May call twice (0→1, 1→2) if still phase 0.
            advance_phase(&self.state, &self.phase, None);
            advance_phase(&self.state, &self.phase, None);
        }
    }
}

wrap_task! {
    pub struct PrimeTask { pub state: Shared }
    impl Task {
        fn execute(&self) {
            let (browser, cdp) = {
                let s = self.state.lock().expect("state poisoned");
                (s.browser.clone(), s.cdp.clone())
            };
            if let Some(h) = browser.lock().expect("browser poisoned")
                .as_ref().and_then(|b| b.host())
            {
                pause_virtual_time(&cdp, &h);
            }
            schedule_step(&self.state);
        }
    }
}

// 0→1 schedules warmup reload; 1→2 (after reload fired) primes capture.
pub fn advance_phase(state: &Shared, phase: &Arc<Mutex<u8>>, iframe: Option<Frame>) {
    let mut p = phase.lock().expect("phase poisoned");
    match *p {
        0 => {
            *p = 1;
            drop(p);
            ipc::set_console_capture(true);
            let mut task = ReloadTask::new(state.clone());
            post_delayed_task(ThreadId::UI, Some(&mut task), WARMUP_MS);
        }
        1 => {
            // Capture only after the warmup reload fired — else it lands mid-capture and deadlocks virtual time.
            if !state.lock().expect("state poisoned").reload_fired { return; }
            *p = 2;
            drop(p);
            // Tolerant drives phase via dom-ready (no frame arg) — resolve the sub-frame by name, not main_frame() (host page).
            let f = iframe.or_else(|| {
                state.lock().expect("state poisoned").browser.lock().expect("browser poisoned")
                    .as_ref().and_then(|b| b.frame_by_name(Some(&CefString::from("havi_target"))))
            });
            if let Some(ff) = f {
                *state.lock().expect("state poisoned").iframe.lock().expect("iframe poisoned") = Some(ff);
            }
            let mut task = PrimeTask::new(state.clone());
            post_task(ThreadId::UI, Some(&mut task));
        }
        _ => {}
    }
}

wrap_load_handler! {
    pub struct DetLoadHandler {
        pub state: Shared,
        pub phase: Arc<Mutex<u8>>,
        pub tolerant: bool,
    }

    impl LoadHandler {
        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _http_status_code: ::std::os::raw::c_int,
        ) {
            if self.tolerant { return; }
            let iframe = match frame { Some(f) if f.is_main() == 0 => f.clone(), _ => return };
            advance_phase(&self.state, &self.phase, Some(iframe));
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let msg = format!(
                "load failed code={:?} text={} url={}",
                error_code,
                error_text.map(|s| s.to_string()).unwrap_or_default(),
                failed_url.map(|s| s.to_string()).unwrap_or_default(),
            );
            ipc::error(&msg);
        }
    }
}
