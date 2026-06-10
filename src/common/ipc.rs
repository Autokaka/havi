// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use indicatif::ProgressBar;
use serde::{Deserialize, Serialize};
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

pub fn human_progress(done: u32, total: u32) {
    let bar = bar_get_or_init(total);
    bar.set_position(done as u64);
    if done >= total { bar.finish(); }
}

fn bar_get_or_init(total: u32) -> ProgressBar {
    static PB: OnceLock<ProgressBar> = OnceLock::new();
    PB.get_or_init(|| {
        let pb = ProgressBar::new(total as u64);
        pb.set_style(
            indicatif::ProgressStyle::with_template("[{bar:40}] {pos}/{len} ({percent}%)")
                .unwrap().progress_chars("#-"),
        );
        set_progress_bar(pb.clone());
        pb
    }).clone()
}

pub fn human_log(line: &str) { log_line(line); }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level { Info, Warn, Error }

pub type RenderId = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Cmd {
    Start { id: RenderId, opts: crate::api::RenderOpts },
    Cancel { id: RenderId },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "evt", rename_all = "snake_case")]
pub enum Evt {
    HostReady,
    Started { id: RenderId },
    Progress { id: RenderId, frame: u32, total: u32 },
    Console { id: RenderId, level: Level, source: String, message: String },
    Done { id: RenderId, out: String, frames: u32, elapsed_ms: u64 },
    Error { id: RenderId, message: String },
    HostExit,
}

pub fn error(message: &str) {
    crate::host::ipc::emit_evt(&Evt::Error { id: 0, message: message.to_string() });
}

pub fn console(level: Level, source: &str, message: &str) {
    crate::host::ipc::emit_evt(&Evt::Console {
        id: 0, level, source: source.to_string(), message: message.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_roundtrip_start() {
        let opts = crate::api::RenderOpts {
            source: "a.html".into(),
            out: Some("o.mp4".into()),
            width: Some(800), height: Some(600),
            fps: Some(30), duration: Some(5),
            tolerant: Some(false), proxy: None,
        };
        let cmd = Cmd::Start { id: 7, opts };
        let line = serde_json::to_string(&cmd).unwrap();
        assert!(line.contains(r#""cmd":"start""#));
        assert!(line.contains(r#""id":7"#));
        let back: Cmd = serde_json::from_str(&line).unwrap();
        assert!(matches!(back, Cmd::Start { id: 7, .. }));
    }

    #[test]
    fn evt_roundtrip_done() {
        let evt = Evt::Done { id: 3, out: "o.mp4".into(), frames: 150, elapsed_ms: 4200 };
        let line = serde_json::to_string(&evt).unwrap();
        assert!(line.contains(r#""evt":"done""#));
        let back: Evt = serde_json::from_str(&line).unwrap();
        if let Evt::Done { id, frames, .. } = back {
            assert_eq!(id, 3);
            assert_eq!(frames, 150);
        } else { panic!("wrong variant"); }
    }

    #[test]
    fn evt_host_ready_tagless() {
        let line = serde_json::to_string(&Evt::HostReady).unwrap();
        assert_eq!(line, r#"{"evt":"host_ready"}"#);
    }
}
