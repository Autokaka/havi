// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::cef::resource::make_handler;
use crate::video::scheme_url::{parse_url, Query};
use crate::video::session;
use cef::*;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

pub const SCHEME: &str = "havi-frame";

pub use session::set_ffmpeg;

type DomReadyHook = Box<dyn Fn() + Send + Sync>;
static ON_DOM_READY: OnceLock<DomReadyHook> = OnceLock::new();

pub fn set_on_dom_ready<F: Fn() + Send + Sync + 'static>(f: F) {
    let _ = ON_DOM_READY.set(Box::new(f));
}

wrap_scheme_handler_factory! {
    pub struct HaviFrameFactory;
    impl SchemeHandlerFactory {
        fn create(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _scheme_name: Option<&CefString>,
            request: Option<&mut Request>,
        ) -> Option<ResourceHandler> {
            let req = request?;
            let url = CefString::from(&req.url()).to_string();
            let q = parse_url(&url, SCHEME).unwrap_or(Query { path: String::new(), params: HashMap::new() });
            Some(route(&q.path, &q.params))
        }
    }
}

fn route(path: &str, params: &HashMap<String, String>) -> ResourceHandler {
    match path {
        "open" => open_route(params),
        "frame" => frame_route(params),
        "close" => close_route(params),
        "dom-ready" => dom_ready_route(),
        _ => make_handler(Vec::new(), "text/plain", 404),
    }
}

fn open_route(params: &HashMap<String, String>) -> ResourceHandler {
    let src = params.get("src").cloned().unwrap_or_default();
    let fps = params.get("fps").and_then(|s| s.parse().ok()).unwrap_or(30);
    match session::open(&src, fps) {
        Some(s) => {
            let json = format!(
                r#"{{"id":"{}","fps":{},"width":{},"height":{},"duration":{}}}"#,
                s.id, s.fps, s.width, s.height, s.duration,
            );
            make_handler(json.into_bytes(), "application/json", 200)
        }
        None => make_handler(Vec::new(), "application/json", 404),
    }
}

fn frame_route(params: &HashMap<String, String>) -> ResourceHandler {
    let id = params.get("id").cloned().unwrap_or_default();
    let idx = params.get("idx").and_then(|s| s.parse().ok()).unwrap_or(1);
    let sess = session::sessions().lock().ok().and_then(|m| m.get(&id).cloned());
    match sess {
        Some(s) => match session::wait_frame(&s, idx, Duration::from_secs(5)) {
            Some(b) => make_handler(b, "image/png", 200),
            None => make_handler(Vec::new(), "image/png", 410),
        },
        None => make_handler(Vec::new(), "image/png", 404),
    }
}

fn close_route(params: &HashMap<String, String>) -> ResourceHandler {
    let id = params.get("id").cloned().unwrap_or_default();
    session::close(&id);
    make_handler(Vec::new(), "text/plain", 200)
}

fn dom_ready_route() -> ResourceHandler {
    if let Some(cb) = ON_DOM_READY.get() { cb(); }
    make_handler(Vec::new(), "text/plain", 204)
}
