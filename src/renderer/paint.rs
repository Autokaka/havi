// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::renderer::capture::{schedule_invalidate, schedule_step, Shared};
use crate::renderer::host::decode_stego;
use crate::ipc;
use cef::*;
use indicatif::ProgressBar;
use std::sync::OnceLock;

static PROGRESS: OnceLock<ProgressBar> = OnceLock::new();

fn draw_progress(done: u32, total: u32) {
    if ipc::enabled() {
        ipc::emit(&ipc::Msg::Progress { frame: done, total });
        return;
    }
    let bar = PROGRESS.get_or_init(|| {
        let pb = ProgressBar::new(total as u64);
        pb.set_style(
            indicatif::ProgressStyle::with_template("[{bar:40}] {pos}/{len} ({percent}%)")
                .unwrap()
                .progress_chars("#-"),
        );
        ipc::set_progress_bar(pb.clone());
        pb
    });
    bar.set_position(done as u64);
    if done >= total { bar.finish(); }
}

wrap_render_handler! {
    pub struct CaptureHandler { pub state: Shared }
    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                let cap = self.state.lock().expect("state poisoned");
                rect.x = 0;
                rect.y = 0;
                rect.width = cap.width;
                rect.height = cap.height + 1;
            }
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            _paint_type: PaintElementType,
            _dirty: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            let cap_state = self.state.clone();
            let mut cap = cap_state.lock().expect("state poisoned");
            if cap.done || buffer.is_null() { return; }
            if !cap.budget_done { return; }
            let buf = unsafe {
                std::slice::from_raw_parts(buffer, (width as usize) * (height as usize) * 4)
            };

            const MAX_INVALIDATES: u32 = 60;
            let stego_match = decode_stego(buf, width, height) == Some(cap.requested_ms);
            if !stego_match {
                cap.stuck_invalidates += 1;
                if cap.stuck_invalidates > MAX_INVALIDATES {
                    let frame = cap.next_frame;
                    let ms = cap.requested_ms;
                    cap.done = true;
                    cap.tx = None;
                    drop(cap);
                    ipc::error(&format!("compositor stalled at frame {frame} ms={ms} — aborting"));
                    quit_message_loop();
                    return;
                }
                let browser = cap.browser.clone();
                drop(cap);
                schedule_invalidate(browser);
                return;
            }

            let stride = (width as usize) * 4;
            let payload = cap.height as usize * stride;
            let n = cap.next_frame;
            let total = cap.total_frames;
            let last = n + 1 >= total;
            let frame_bytes = buf[..payload].to_vec();
            // Encoder gone (ffmpeg died) — quit cleanly so main reports its exit
            // status as an error, instead of panicking (silent abort, no event).
            if let Some(tx) = cap.tx.as_ref() {
                if tx.send(frame_bytes).is_err() {
                    cap.done = true;
                    cap.tx = None;
                    drop(cap);
                    quit_message_loop();
                    return;
                }
            }
            draw_progress(n + 1, total);

            if last {
                cap.done = true;
                cap.tx = None;
                drop(cap);
                quit_message_loop();
            } else {
                cap.next_frame = n + 1;
                drop(cap);
                schedule_step(&cap_state);
            }
        }
    }
}
