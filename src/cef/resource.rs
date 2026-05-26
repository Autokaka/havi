use cef::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub fn make_handler(bytes: Vec<u8>, mime: &str, status: u16) -> ResourceHandler {
    SyntheticResource::new(
        Arc::new(bytes),
        mime.to_string(),
        status,
        Arc::new(HashMap::new()),
        Arc::new(AtomicUsize::new(0)),
    )
}

pub fn make_handler_with_headers(
    bytes: Vec<u8>,
    mime: &str,
    status: u16,
    headers: HashMap<String, String>,
) -> ResourceHandler {
    SyntheticResource::new(
        Arc::new(bytes),
        mime.to_string(),
        status,
        Arc::new(headers),
        Arc::new(AtomicUsize::new(0)),
    )
}

wrap_resource_handler! {
    pub struct SyntheticResource {
        pub bytes: Arc<Vec<u8>>,
        pub mime: String,
        pub status: u16,
        pub headers: Arc<HashMap<String, String>>,
        pub pos: Arc<AtomicUsize>,
    }
    impl ResourceHandler {
        fn open(
            &self,
            _request: Option<&mut Request>,
            handle_request: Option<&mut ::std::os::raw::c_int>,
            _callback: Option<&mut Callback>,
        ) -> ::std::os::raw::c_int {
            if let Some(h) = handle_request { *h = 1; }
            1
        }

        fn response_headers(
            &self,
            response: Option<&mut Response>,
            response_length: Option<&mut i64>,
            _redirect_url: Option<&mut CefString>,
        ) {
            if let Some(r) = response {
                r.set_status(self.status as i32);
                r.set_mime_type(Some(&CefString::from(self.mime.as_str())));
                r.set_header_by_name(
                    Some(&CefString::from("Access-Control-Allow-Origin")),
                    Some(&CefString::from("*")),
                    1,
                );
                for (k, v) in self.headers.iter() {
                    r.set_header_by_name(
                        Some(&CefString::from(k.as_str())),
                        Some(&CefString::from(v.as_str())),
                        1,
                    );
                }
            }
            if let Some(len) = response_length {
                *len = i64::try_from(self.bytes.len()).unwrap_or(i64::MAX);
            }
        }

        fn read(
            &self,
            data_out: *mut u8,
            bytes_to_read: ::std::os::raw::c_int,
            bytes_read: Option<&mut ::std::os::raw::c_int>,
            _callback: Option<&mut ResourceReadCallback>,
        ) -> ::std::os::raw::c_int {
            let pos = self.pos.load(Ordering::SeqCst);
            let remaining = self.bytes.len().saturating_sub(pos);
            let want = usize::try_from(bytes_to_read.max(0)).unwrap_or(0);
            let n = remaining.min(want);
            if n == 0 {
                if let Some(br) = bytes_read { *br = 0; }
                return 0;
            }
            unsafe { std::ptr::copy_nonoverlapping(self.bytes.as_ptr().add(pos), data_out, n); }
            self.pos.store(pos + n, Ordering::SeqCst);
            if let Some(br) = bytes_read {
                *br = i32::try_from(n).unwrap_or(i32::MAX);
            }
            1
        }
    }
}
