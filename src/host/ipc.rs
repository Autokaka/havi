// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

use crate::ipc::{Cmd, Evt};
use std::io::Write;

pub fn emit_evt(evt: &Evt) {
    let Ok(line) = serde_json::to_string(evt) else { return };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
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
