// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::renderer::flags::deterministic_flags;
use crate::video::scheme;
use cef::*;

const IFRAME_HOOK: &str = include_str!("../runtime/iframe_hook.js");
const IFRAME_VIDEO_HOOK: &str = include_str!("../runtime/iframe_video_hook.js");
const DOM_READY_HOOK: &str = include_str!("../runtime/dom_ready_hook.js");

wrap_render_process_handler! {
    pub struct DetRenderProc;

    impl RenderProcessHandler {
        fn on_context_created(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _context: Option<&mut V8Context>,
        ) {
            // Hook only the user sub-frame, never havi's host page — else the host
            // also fires dom-ready, double-advancing the phase before warmup reload.
            if let Some(frame) = frame.filter(|f| f.is_main() == 0) {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                let hook = IFRAME_HOOK.replace("__HAVI_DATE_ORIGIN__", &now_ms.to_string());
                frame.execute_java_script(
                    Some(&CefString::from(hook.as_str())),
                    Some(&CefString::from("iframe_hook.js")),
                    0,
                );
                frame.execute_java_script(
                    Some(&CefString::from(IFRAME_VIDEO_HOOK)),
                    Some(&CefString::from("iframe_video_hook.js")),
                    0,
                );
                frame.execute_java_script(
                    Some(&CefString::from(DOM_READY_HOOK)),
                    Some(&CefString::from("dom_ready_hook.js")),
                    0,
                );
            }
        }
    }
}

wrap_app! {
    pub struct DetApp;

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            if let Some(cmd) = command_line {
                for flag in deterministic_flags() {
                    // key=value needs a value switch, else stored under key "k=v" and GetSwitchValue("k") misses it.
                    match flag.split_once('=') {
                        Some((k, v)) => cmd.append_switch_with_value(
                            Some(&CefString::from(k)), Some(&CefString::from(v))),
                        None => cmd.append_switch(Some(&CefString::from(flag.as_str()))),
                    }
                }
            }
        }

        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(DetRenderProc::new())
        }

        fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
            if let Some(reg) = registrar {
                let opts = SchemeOptions::STANDARD.get_raw()
                    | SchemeOptions::CORS_ENABLED.get_raw()
                    | SchemeOptions::FETCH_ENABLED.get_raw();
                reg.add_custom_scheme(
                    Some(&CefString::from(scheme::SCHEME)),
                    opts as ::std::os::raw::c_int,
                );
            }
        }
    }
}

pub fn make_app() -> App {
    DetApp::new()
}

/// Must run before any CEF object — else "invalid version -1".
pub fn init_api() {
    let _ = api_hash(cef::sys::CEF_API_VERSION_LAST, 0);

    #[cfg(target_os = "macos")]
    crate::cef::mac::setup_application();
}
