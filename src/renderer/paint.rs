// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::host::ipc::emit_evt;
use crate::host::render::Host;
use crate::ipc::{self, Evt};
use crate::renderer::capture::schedule_invalidate;
use crate::renderer::host::decode_stego;
use cef::*;
use std::sync::Arc;

fn report_progress(id: u64, frame: u32, total: u32) {
    emit_evt(&Evt::Progress { id, frame, total });
}

wrap_render_handler! {
    pub struct CaptureHandler { pub host: Arc<Host> }
    impl RenderHandler {
        fn view_rect(&self, browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let (Some(browser), Some(rect)) = (browser, rect) else { return };
            let Some(render) = self.host.by_browser(browser.identifier())
                .or_else(|| self.host.creating()) else { return };
            let r = render.lock().expect("render poisoned");
            rect.x = 0;
            rect.y = 0;
            rect.width = r.width;
            rect.height = r.height + 1;
        }

        fn on_paint(
            &self,
            browser: Option<&mut Browser>,
            _paint_type: PaintElementType,
            _dirty: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            let Some(browser) = browser else { return };
            let Some(render) = self.host.by_browser(browser.identifier()) else { return };
            let mut r = render.lock().expect("render poisoned");
            if r.done || buffer.is_null() { return; }
            if !r.capture.budget_done { return; }
            let buf = unsafe {
                std::slice::from_raw_parts(buffer, (width as usize) * (height as usize) * 4)
            };

            const MAX_INVALIDATES: u32 = 60;
            let decoded = decode_stego(buf, width, height);
            let stego_match = decoded == Some(r.capture.requested_ms);
            if !stego_match {
                r.capture.stuck_invalidates += 1;
                emit_evt(&Evt::Console {
                    id: r.id, level: ipc::Level::Warn, source: "havi".into(),
                    message: format!(
                        "stuck frame={} req={} got={:?} retry={}/{}",
                        r.capture.next_frame, r.capture.requested_ms, decoded,
                        r.capture.stuck_invalidates, MAX_INVALIDATES
                    ),
                });
                if r.capture.stuck_invalidates > MAX_INVALIDATES {
                    let id = r.id;
                    let frame = r.capture.next_frame;
                    let ms = r.capture.requested_ms;
                    r.done = true;
                    r.errored = true;
                    r.tx = None;
                    drop(r);
                    emit_evt(&Evt::Error {
                        id,
                        message: format!("compositor stalled at frame {frame} ms={ms}"),
                    });
                    self.host.finish(id);
                    return;
                }
                let browser = r.browser.clone();
                drop(r);
                schedule_invalidate(browser);
                return;
            }

            let stride = (width as usize) * 4;
            let payload = r.height as usize * stride;
            let n = r.capture.next_frame;
            let total = r.total_frames;
            let last = n + 1 >= total;
            let frame_bytes = buf[..payload].to_vec();
            if let Some(tx) = r.tx.as_ref() {
                let _ = tx.send(frame_bytes);
            }
            let id = r.id;
            report_progress(id, n + 1, total);

            if last {
                r.capture.next_frame = n + 1;
                r.done = true;
                r.tx = None;
                drop(r);
                self.host.finish(id);
            } else {
                r.capture.next_frame = n + 1;
                let render2 = render.clone();
                drop(r);
                crate::renderer::capture::schedule_step(&render2);
            }
        }
    }
}
