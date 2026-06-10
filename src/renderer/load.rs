// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::host::ipc::emit_evt;
use crate::host::render::{Host, RenderRef};
use crate::ipc::{self, Evt, RenderId};
use crate::renderer::capture::{pause_virtual_time, schedule_step};
use cef::*;
use std::sync::Arc;

const WARMUP_MS: i64 = 2000;
pub const LOAD_TIMEOUT_MS: i64 = 15000;

wrap_task! {
    pub struct ReloadTask { pub browser: crate::renderer::capture::BrowserHandle }
    impl Task {
        fn execute(&self) {
            if let Some(main) = self.browser.lock().expect("browser poisoned")
                .as_ref().and_then(|b| b.main_frame())
            {
                let url = CefString::from(&main.url()).to_string();
                main.load_url(Some(&CefString::from(url.as_str())));
            }
        }
    }
}

wrap_task! {
    pub struct TolerantTimeoutTask { pub host: Arc<Host>, pub id: RenderId }
    impl Task {
        fn execute(&self) {
            let Some(render) = self.host.by_id(self.id) else { return };
            if render.lock().expect("render poisoned").phase >= 2 { return; }
            let tolerant = render.lock().expect("render poisoned").tolerant;
            if !tolerant {
                render.lock().expect("render poisoned").errored = true;
                emit_evt(&Evt::Error {
                    id: self.id,
                    message: format!("load timeout after {LOAD_TIMEOUT_MS}ms (pass tolerant to proceed anyway)"),
                });
                self.host.finish(self.id);
                return;
            }
            emit_evt(&Evt::Console {
                id: self.id, level: ipc::Level::Warn, source: "havi".into(),
                message: format!("load timeout after {LOAD_TIMEOUT_MS}ms — proceeding tolerant"),
            });
            advance_phase(&self.host, &render, None);
            advance_phase(&self.host, &render, None);
        }
    }
}

wrap_task! {
    pub struct PrimeTask { pub render: RenderRef }
    impl Task {
        fn execute(&self) {
            let (browser, cdp) = {
                let r = self.render.lock().expect("render poisoned");
                (r.browser.clone(), r.cdp.clone())
            };
            if let Some(h) = browser.lock().expect("browser poisoned")
                .as_ref().and_then(|b| b.host())
            {
                pause_virtual_time(&cdp, &h);
            }
            schedule_step(&self.render);
        }
    }
}

pub fn advance_phase(host: &Arc<Host>, render: &RenderRef, iframe: Option<Frame>) {
    let phase = render.lock().expect("render poisoned").phase;
    match phase {
        0 => {
            render.lock().expect("render poisoned").phase = 1;
            ipc::set_console_capture(true);
            let browser = render.lock().expect("render poisoned").browser.clone();
            let mut task = ReloadTask::new(browser);
            post_delayed_task(ThreadId::UI, Some(&mut task), WARMUP_MS);
        }
        1 => {
            render.lock().expect("render poisoned").phase = 2;
            let f = iframe.or_else(|| {
                render.lock().expect("render poisoned").browser.lock().expect("browser")
                    .as_ref().and_then(|b| b.main_frame())
            });
            if let Some(ff) = f {
                *render.lock().expect("render poisoned").iframe.lock().expect("iframe") = Some(ff);
            }
            let _ = host;
            let mut task = PrimeTask::new(render.clone());
            post_task(ThreadId::UI, Some(&mut task));
        }
        _ => {}
    }
}

wrap_load_handler! {
    pub struct DetLoadHandler { pub host: Arc<Host> }

    impl LoadHandler {
        fn on_load_end(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _http_status_code: ::std::os::raw::c_int,
        ) {
            let Some(browser) = browser else { return };
            let Some(render) = self.host.by_browser(browser.identifier()) else { return };
            if render.lock().expect("render poisoned").tolerant { return; }
            let iframe = match frame { Some(f) if f.is_main() == 0 => f.clone(), _ => return };
            advance_phase(&self.host, &render, Some(iframe));
        }

        fn on_load_error(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let Some(browser) = browser else { return };
            let Some(render) = self.host.by_browser(browser.identifier()) else { return };
            let id = render.lock().expect("render poisoned").id;
            let msg = format!(
                "load failed code={:?} text={} url={}",
                error_code,
                error_text.map(|s| s.to_string()).unwrap_or_default(),
                failed_url.map(|s| s.to_string()).unwrap_or_default(),
            );
            emit_evt(&Evt::Error { id, message: msg });
            if _frame.map(|f| f.is_main() != 0).unwrap_or(false) {
                render.lock().expect("render poisoned").errored = true;
                self.host.finish(id);
            }
        }
    }
}
