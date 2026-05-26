use crate::{ipc, cef::perm::DenyAll, proxy::{Compiled, ProxyReqHandler}};
use cef::*;
use std::sync::Arc;

fn map_level(sev: i64) -> ipc::Level {
    if sev >= LogSeverity::ERROR.get_raw() as i64 { ipc::Level::Error }
    else if sev >= LogSeverity::WARNING.get_raw() as i64 { ipc::Level::Warn }
    else { ipc::Level::Info }
}

wrap_display_handler! {
    pub struct ConsoleForward;
    impl DisplayHandler {
        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            level: LogSeverity,
            message: Option<&CefString>,
            source: Option<&CefString>,
            line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            if !ipc::console_capture_enabled() { return 0; }
            let m = message.map(|s| s.to_string()).unwrap_or_default();
            let src = source.map(|s| s.to_string()).unwrap_or_default();
            let where_s = format!("{src}:{line}");
            ipc::console(map_level(level.get_raw() as i64), &where_s, &m);
            0
        }
    }
}

wrap_client! {
    pub struct DetClient {
        pub render: RenderHandler,
        pub load: LoadHandler,
        pub proxy: Option<Arc<Compiled>>,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.render.clone())
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(self.load.clone())
        }
        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(DenyAll::new())
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(ConsoleForward::new())
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            self.proxy.as_ref().map(|c| ProxyReqHandler::new(c.clone()))
        }
    }
}
