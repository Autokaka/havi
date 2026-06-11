// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::ipc;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub fn add_input(cmd: &mut Command, src: &str) -> bool {
    if let Some(p) = src.strip_prefix("file://") {
        let path = PathBuf::from(p);
        if !path.exists() { return false; }
        cmd.arg(path);
        true
    } else if src.starts_with("http://") || src.starts_with("https://") {
        cmd.arg(src);
        true
    } else {
        false
    }
}

pub fn probe(ff: &Path, src: &str) -> Option<(u32, u32, f64)> {
    let mut cmd = Command::new(ff);
    cmd.args(["-hide_banner", "-i"]);
    if !add_input(&mut cmd, src) { return None; }
    let out = cmd.output().ok()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut w = 0u32; let mut h = 0u32; let mut dur = 0.0_f64;
    for line in stderr.lines() {
        if let Some(rest) = line.split_once("Duration:") {
            let t = rest.1.trim().split(',').next().unwrap_or("");
            let parts: Vec<&str> = t.split(':').collect();
            if parts.len() == 3 {
                let hh: f64 = parts[0].parse().unwrap_or(0.0);
                let mm: f64 = parts[1].parse().unwrap_or(0.0);
                let ss: f64 = parts[2].parse().unwrap_or(0.0);
                dur = hh * 3600.0 + mm * 60.0 + ss;
            }
        }
        if line.contains("Video:") && w == 0 {
            for tok in line.split(',') {
                if let Some((wp, hp)) = tok.trim().split_once('x') {
                    let ws = trailing_digits(wp);
                    let hs = leading_digits(hp);
                    if let (Ok(pw), Ok(ph)) = (ws.parse::<u32>(), hs.parse::<u32>()) {
                        if pw > 0 && ph > 0 { w = pw; h = ph; break; }
                    }
                }
            }
        }
    }
    if w == 0 || h == 0 { return None; }
    Some((w, h, dur))
}

fn leading_digits(s: &str) -> &str {
    &s[..s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len())]
}

fn trailing_digits(s: &str) -> &str {
    &s[s.rfind(|c: char| !c.is_ascii_digit()).map_or(0, |i| i + 1)..]
}

pub fn count_pngs(dir: &Path) -> u32 {
    std::fs::read_dir(dir)
        .map(|r| r.filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("png"))
            .count() as u32)
        .unwrap_or(0)
}

pub fn run(
    ff: PathBuf, src: String, dir: PathBuf, fps: u32,
    ready: Arc<(Mutex<u32>, Condvar)>, done: Arc<Mutex<bool>>,
) {
    let pattern = dir.join("%06d.png");
    let mut cmd = Command::new(&ff);
    cmd.args(["-loglevel", "error", "-y", "-i"]);
    if !add_input(&mut cmd, &src) {
        ipc::error(&format!("ffmpeg input rejected: {src}"));
        return;
    }
    cmd.args([
        "-vf", &format!("fps={fps}"),
        "-pix_fmt", "rgba",
        "-compression_level", "1",
        "-pred", "none",
    ])
    .arg(&pattern)
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    crate::sandbox::prepare_child(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => { ipc::error(&format!("ffmpeg spawn failed: {e} ({src})")); return; }
    };
    crate::sandbox::track_child(&child);
    let pid = child.id();
    let dir_w = dir.clone();
    let ready_w = ready.clone();
    let stop = Arc::new(Mutex::new(false));
    let stop_w = stop.clone();
    let watcher = std::thread::spawn(move || {
        while !*stop_w.lock().unwrap() {
            let count = count_pngs(&dir_w);
            let mut g = ready_w.0.lock().unwrap();
            if count > *g { *g = count; ready_w.1.notify_all(); }
            drop(g);
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let stderr_pipe = child.stderr.take();
    let status = child.wait();
    crate::sandbox::unregister_ffmpeg(pid);
    *stop.lock().unwrap() = true;
    let _ = watcher.join();
    if let Some(mut p) = stderr_pipe {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = p.read_to_end(&mut buf);
        if !buf.is_empty() {
            let msg = String::from_utf8_lossy(&buf);
            ipc::console(ipc::Level::Warn, &format!("ffmpeg [{src}]"), msg.trim());
        }
    }
    if let Ok(st) = status {
        if !st.success() { ipc::error(&format!("ffmpeg exit {st} for {src}")); }
    }
    let final_count = count_pngs(&dir);
    let mut g = ready.0.lock().unwrap();
    *g = final_count;
    ready.1.notify_all();
    *done.lock().unwrap() = true;
}
