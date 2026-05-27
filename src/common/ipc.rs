// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use indicatif::ProgressBar;
use serde::{Deserialize, Serialize};
use std::io::{stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

pub const ENV_FLAG: &str = "HAVI_IPC";

pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var(ENV_FLAG).map(|v| v == "1").unwrap_or(false))
}

static CONSOLE_CAPTURE: AtomicBool = AtomicBool::new(false);
pub fn console_capture_enabled() -> bool { CONSOLE_CAPTURE.load(Ordering::Relaxed) }
pub fn set_console_capture(on: bool) { CONSOLE_CAPTURE.store(on, Ordering::Relaxed); }

// Shared progress bar (human mode). Logs route through bar.println() so they
// stack above the bar without breaking its redraw.
static BAR: OnceLock<Mutex<Option<ProgressBar>>> = OnceLock::new();
pub fn set_progress_bar(bar: ProgressBar) {
    let _ = BAR.get_or_init(|| Mutex::new(None)).lock().map(|mut g| *g = Some(bar));
}
fn log_line(line: &str) {
    if let Some(Some(bar)) = BAR.get().and_then(|m| m.lock().ok().map(|g| g.clone())) {
        bar.println(line);
    } else {
        eprintln!("{line}");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level { Info, Warn, Error }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Msg {
    Progress { frame: u32, total: u32 },
    Console { level: Level, source: String, message: String },
    Done {
        frames: u32,
        width: i32,
        height: i32,
        fps: u32,
        out: String,
        elapsed_ms: u64,
    },
    Error { message: String },
}

pub fn emit(msg: &Msg) {
    if !enabled() { return; }
    let Ok(line) = serde_json::to_string(msg) else { return };
    let mut s = stdout().lock();
    let _ = writeln!(s, "{line}");
    let _ = s.flush();
}

pub fn error(message: &str) {
    if enabled() {
        emit(&Msg::Error { message: message.to_string() });
    } else {
        log_line(&format!("error: {message}"));
    }
}

pub fn console(level: Level, source: &str, message: &str) {
    if enabled() {
        emit(&Msg::Console { level, source: source.to_string(), message: message.to_string() });
    } else {
        let _ = source;
        let lvl = match level { Level::Info => "info", Level::Warn => "warn", Level::Error => "error" };
        log_line(&format!("{lvl}: {message}"));
    }
}
