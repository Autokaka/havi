// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

use crate::ipc::{self, Cmd, Evt};
use std::io::Write;

pub fn emit_evt(evt: &Evt) {
    if ipc::enabled() {
        let Ok(line) = serde_json::to_string(evt) else { return };
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
        return;
    }
    human_render(evt);
}

fn human_render(evt: &Evt) {
    match evt {
        Evt::Progress { frame, total, .. } => ipc::human_progress(*frame, *total),
        Evt::Console { level, message, .. } => {
            let lvl = match level { ipc::Level::Info => "info", ipc::Level::Warn => "warn", ipc::Level::Error => "error" };
            ipc::human_log(&format!("{lvl}: {message}"));
        }
        Evt::Error { message, .. } => ipc::human_log(&format!("error: {message}")),
        Evt::Done { out, frames, elapsed_ms, .. } => {
            ipc::human_log(&format!("done: {out} ({frames} frames) in {:.2}s", *elapsed_ms as f64 / 1000.0));
        }
        Evt::Started { .. } | Evt::HostReady | Evt::HostExit => {}
    }
}

pub fn parse_cmd(line: &str) -> Option<Cmd> {
    let line = line.trim();
    if line.is_empty() { return None; }
    serde_json::from_str(line).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_start_line() {
        let line = r#"{"cmd":"start","id":1,"opts":{"source":"a.html"}}"#;
        let cmd = parse_cmd(line).expect("parse");
        assert!(matches!(cmd, Cmd::Start { id: 1, .. }));
    }

    #[test]
    fn parse_blank_is_none() {
        assert!(parse_cmd("   ").is_none());
    }

    #[test]
    fn parse_garbage_is_none() {
        assert!(parse_cmd("not json").is_none());
    }
}
