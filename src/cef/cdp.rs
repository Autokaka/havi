use cef::*;
use std::sync::{
    atomic::{AtomicI32, Ordering},
    Arc, Mutex,
};

pub type EventHandler = Arc<dyn Fn(&[u8]) + Send + Sync>;

pub struct CdpInner {
    next_id: AtomicI32,
    event_handlers: Mutex<Vec<(String, EventHandler)>>,
}

#[derive(Clone)]
pub struct Cdp(pub Arc<CdpInner>);

impl Cdp {
    pub fn new() -> Self {
        Self(Arc::new(CdpInner {
            next_id: AtomicI32::new(1),
            event_handlers: Mutex::new(Vec::new()),
        }))
    }

    pub fn on_event<F>(&self, method: &str, handler: F)
    where
        F: Fn(&[u8]) + Send + Sync + 'static,
    {
        self.0
            .event_handlers
            .lock()
            .expect("event handlers poisoned")
            .push((method.to_string(), Arc::new(handler)));
    }

    pub fn send(&self, host: &BrowserHost, method: &str, params_json: &str) {
        let id = self.0.next_id.fetch_add(1, Ordering::Relaxed);
        let msg = format!(r#"{{"id":{id},"method":"{method}","params":{params_json}}}"#);
        host.send_dev_tools_message(Some(msg.as_bytes()));
    }
}

wrap_dev_tools_message_observer! {
    pub struct CdpObserver {
        pub cdp: Cdp,
    }
    impl DevToolsMessageObserver {
        fn on_dev_tools_event(
            &self,
            _browser: Option<&mut Browser>,
            method: Option<&CefString>,
            params: Option<&[u8]>,
        ) {
            let Some(method) = method else { return };
            let method = method.to_string();
            let p = params.unwrap_or(&[]);
            let handlers = self.cdp.0.event_handlers.lock().expect("event handlers poisoned");
            for (m, h) in handlers.iter() {
                if m == &method { h(p); }
            }
        }
        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut Browser>,
            message_id: ::std::os::raw::c_int,
            success: ::std::os::raw::c_int,
            result: Option<&[u8]>,
        ) {
            if success == 0 {
                let text = result.map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                eprintln!("\nerror: cdp id={message_id}: {text}");
            }
        }
    }
}
