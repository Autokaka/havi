// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::ipc;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread::JoinHandle;

pub fn ffmpeg_path() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let dir = exe.parent().expect("exe parent");
    if cfg!(windows) { dir.join("ffmpeg.exe") } else { dir.join("ffmpeg") }
}

// .webm → VP9+alpha, else → HEVC+alpha mp4 (libx265 ENABLE_ALPHA, bit-exact).
fn codec_args(out: &str, x265: &str) -> Vec<String> {
    let webm = Path::new(out).extension().is_some_and(|e| e.eq_ignore_ascii_case("webm"));
    let raw: &[&str] = if webm {
        &["-c:v", "libvpx-vp9", "-pix_fmt", "yuva420p", "-b:v", "4M",
          "-deadline", "realtime", "-cpu-used", "8", "-row-mt", "1"]
    } else {
        &["-c:v", "libx265", "-preset", "fast", "-crf", "23", "-pix_fmt", "yuva420p",
          "-tag:v", "hvc1", "-x265-params", x265, "-movflags", "+faststart"]
    };
    raw.iter().map(|s| s.to_string()).collect()
}

pub fn spawn(width: i32, height: i32, fps: u32, outs: &[String]) -> Child {
    let ffmpeg = ffmpeg_path();
    let fps_str = fps.to_string();
    let size = format!("{width}x{height}");
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).to_string();
    let x265_params = format!("log-level=1:alpha=1:pools={threads}:frame-threads={threads}:no-info=1");

    let mut args: Vec<String> = ["-loglevel", "error", "-y", "-f", "rawvideo",
        "-pixel_format", "bgra", "-video_size", &size, "-framerate", &fps_str, "-i", "-"]
        .iter().map(|s| s.to_string()).collect();
    for out in outs {
        args.extend(codec_args(out, &x265_params));
        args.push(out.clone());
    }

    let mut cmd = Command::new(ffmpeg);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    crate::sandbox::prepare_child(&mut cmd);
    let mut child = cmd.spawn().expect("failed to start ffmpeg");
    crate::sandbox::track_child(&child);

    if let Some(stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line.is_empty() { continue; }
                ipc::console(ipc::Level::Warn, "ffmpeg", &line);
            }
        });
    }
    child
}

const QUEUE_DEPTH: usize = 8;

pub fn start_pipe(child: &mut Child) -> (SyncSender<Vec<u8>>, JoinHandle<()>) {
    let stdin = child.stdin.take().expect("ffmpeg stdin");
    let (tx, rx): (SyncSender<Vec<u8>>, Receiver<Vec<u8>>) = sync_channel(QUEUE_DEPTH);
    let handle = std::thread::spawn(move || {
        let mut stdin = stdin;
        while let Ok(buf) = rx.recv() {
            if stdin.write_all(&buf).is_err() { break; }
        }
    });
    (tx, handle)
}
