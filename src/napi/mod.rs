// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::{api, cli, ipc};
use napi::bindgen_prelude::*;
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

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
    pid: AtomicU32,
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
                let p = state.pid.load(Ordering::SeqCst);
                if p > 0 { cancel_pid(p); }
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
    let mut handle = api::spawn(input.options).map_err(|e| Error::from_reason(e.to_string()))?;
    let pid = handle.pid();
    input.abort_state.pid.store(pid, Ordering::SeqCst);
    if input.abort_state.flag.load(Ordering::SeqCst) {
        cancel_pid(pid);
    }
    let abort_state = input.abort_state.clone();
    let on_progress = input.on_progress;
    let on_console = input.on_console;

    env.spawn_future(async move {
        loop {
            let msg = napi::tokio::task::block_in_place(|| handle.recv());
            match msg {
                Some(ipc::Msg::Progress { frame, total }) => {
                    if let Some(cb) = &on_progress {
                        cb.call(ProgressEvent { frame, total }, ThreadsafeFunctionCallMode::NonBlocking);
                    }
                }
                Some(ipc::Msg::Console { level, source, message }) => {
                    if let Some(cb) = &on_console {
                        let level = match level {
                            ipc::Level::Info => "info",
                            ipc::Level::Warn => "warn",
                            ipc::Level::Error => "error",
                        }.to_string();
                        cb.call(ConsoleEvent { level, source, message }, ThreadsafeFunctionCallMode::NonBlocking);
                    }
                }
                Some(ipc::Msg::Done { frames, width, height, fps, out, elapsed_ms }) => {
                    return Ok(RenderResult {
                        frames, width, height, fps, out,
                        elapsed_ms: u32::try_from(elapsed_ms).unwrap_or(u32::MAX),
                    });
                }
                Some(ipc::Msg::Error { message }) => return Err(Error::from_reason(message)),
                None => {
                    if abort_state.flag.load(Ordering::SeqCst) {
                        return Err(Error::from_reason("aborted"));
                    }
                    return Err(Error::from_reason("render exited without done"));
                }
            }
        }
    })
}

fn cancel_pid(pid: u32) {
    #[cfg(unix)]
    {
        let pid_i32 = i32::try_from(pid).unwrap_or(0);
        if pid_i32 > 0 { unsafe { libc::kill(pid_i32, libc::SIGTERM); } }
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).status();
    }
}
