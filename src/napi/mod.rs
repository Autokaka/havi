// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::api::HostClient;
use crate::ipc::Evt;
use crate::{api, cli, ipc};
use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

static HOST: OnceLock<Arc<HostClient>> = OnceLock::new();

fn host() -> Result<Arc<HostClient>> {
    if let Some(h) = HOST.get() { return Ok(h.clone()); }
    let h = HostClient::spawn().map_err(|e| Error::from_reason(e.to_string()))?;
    let _ = HOST.set(h.clone());
    Ok(HOST.get().cloned().unwrap_or(h))
}

#[napi]
pub fn render_help() -> String {
    cli::render_help()
}

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

type ProgressTsfn = ThreadsafeFunction<ProgressEvent, (), ProgressEvent, Status, false>;
type ConsoleTsfn = ThreadsafeFunction<ConsoleEvent, (), ConsoleEvent, Status, false>;

#[derive(Default)]
struct AbortState {
    flag: AtomicBool,
    id: AtomicU64,
}

pub struct RenderInput {
    pub options: api::RenderOpts,
    pub on_progress: Option<ProgressTsfn>,
    pub on_console: Option<ConsoleTsfn>,
    abort_state: Arc<AbortState>,
}

impl FromNapiValue for RenderInput {
    unsafe fn from_napi_value(env: napi::sys::napi_env, val: napi::sys::napi_value) -> Result<Self> {
        let obj = unsafe { Object::from_napi_value(env, val) }?;
        let options = obj.get::<api::RenderOpts>("options")?
            .ok_or_else(|| Error::from_reason("missing options"))?;
        let on_progress = obj.get::<ProgressTsfn>("onProgress")?;
        let on_console = obj.get::<ConsoleTsfn>("onConsole")?;
        let signal = obj.get::<AbortSignal>("signal")?;
        let abort_state = Arc::new(AbortState::default());
        if let Some(s) = signal {
            let state = abort_state.clone();
            s.on_abort(move || {
                state.flag.store(true, Ordering::SeqCst);
                let id = state.id.load(Ordering::SeqCst);
                if id > 0 {
                    if let Some(h) = HOST.get() { h.cancel(id); }
                }
            });
        }
        Ok(Self { options, on_progress, on_console, abort_state })
    }
}

impl TypeName for RenderInput {
    fn type_name() -> &'static str { "RenderInput" }
    fn value_type() -> ValueType { ValueType::Object }
}

impl ValidateNapiValue for RenderInput {}

#[napi]
pub fn render<'env>(
    env: &'env Env,
    input: RenderInput,
) -> Result<PromiseRaw<'env, RenderResult>> {
    let client = host()?;
    let opts = input.options;
    let (rw, rh, rfps) = (opts.width_or(), opts.height_or(), opts.fps_or());
    let (id, rx) = client.begin(opts).map_err(|e| Error::from_reason(e.to_string()))?;
    input.abort_state.id.store(id, Ordering::SeqCst);
    if input.abort_state.flag.load(Ordering::SeqCst) {
        client.cancel(id);
    }
    let _abort_state = input.abort_state.clone();
    let on_progress = input.on_progress;
    let on_console = input.on_console;
    let client2 = client.clone();

    env.spawn_future(async move {
        let result = loop {
            let evt = napi::tokio::task::block_in_place(|| rx.recv().ok());
            match evt {
                Some(Evt::Progress { frame, total, .. }) => {
                    if let Some(cb) = &on_progress {
                        cb.call(ProgressEvent { frame, total }, ThreadsafeFunctionCallMode::NonBlocking);
                    }
                }
                Some(Evt::Console { level, source, message, .. }) => {
                    if let Some(cb) = &on_console {
                        let level = match level {
                            ipc::Level::Info => "info",
                            ipc::Level::Warn => "warn",
                            ipc::Level::Error => "error",
                        }.to_string();
                        cb.call(ConsoleEvent { level, source, message }, ThreadsafeFunctionCallMode::NonBlocking);
                    }
                }
                Some(Evt::Started { .. }) => {}
                Some(Evt::Done { frames, out, elapsed_ms, .. }) => {
                    break Ok(RenderResult {
                        frames,
                        width: rw, height: rh, fps: rfps,
                        out,
                        elapsed_ms: u32::try_from(elapsed_ms).unwrap_or(u32::MAX),
                    });
                }
                Some(Evt::Error { message, .. }) => break Err(Error::from_reason(message)),
                Some(Evt::HostReady) | Some(Evt::HostExit) => {}
                None => break Err(Error::from_reason("render channel closed")),
            }
        };
        client2.forget(id);
        result
    })
}
