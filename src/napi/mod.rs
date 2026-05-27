// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::{api, ipc};
use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};

#[napi(object)]
pub struct RenderResult {
    pub frames: u32,
    pub width: i32,
    pub height: i32,
    pub fps: u32,
    pub out: String,
    pub elapsed_ms: u32,
}

#[napi(object)]
pub struct ProgressEvent {
    pub frame: u32,
    pub total: u32,
}

#[napi(object)]
pub struct ConsoleEvent {
    pub level: String,
    pub source: String,
    pub message: String,
}

#[napi]
pub async fn render(
    opts: api::RenderOpts,
    on_progress: Option<ThreadsafeFunction<ProgressEvent>>,
    on_console: Option<ThreadsafeFunction<ConsoleEvent>>,
) -> Result<RenderResult> {
    let mut handle = api::spawn(opts).map_err(|e| Error::from_reason(e.to_string()))?;
    while let Some(msg) = handle.recv() {
        match msg {
            ipc::Msg::Progress { frame, total } => {
                if let Some(cb) = &on_progress {
                    cb.call(Ok(ProgressEvent { frame, total }), ThreadsafeFunctionCallMode::NonBlocking);
                }
            }
            ipc::Msg::Console { level, source, message } => {
                if let Some(cb) = &on_console {
                    let level = match level {
                        ipc::Level::Info => "info",
                        ipc::Level::Warn => "warn",
                        ipc::Level::Error => "error",
                    }.to_string();
                    cb.call(Ok(ConsoleEvent { level, source, message }), ThreadsafeFunctionCallMode::NonBlocking);
                }
            }
            ipc::Msg::Done { frames, width, height, fps, out, elapsed_ms } => {
                return Ok(RenderResult {
                    frames, width, height, fps, out,
                    elapsed_ms: u32::try_from(elapsed_ms).unwrap_or(u32::MAX),
                });
            }
            ipc::Msg::Error { message } => return Err(Error::from_reason(message)),
        }
    }
    Err(Error::from_reason("render exited without done"))
}
