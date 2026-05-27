// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::ipc;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread::JoinHandle;

pub fn ffmpeg_path() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let dir = exe.parent().expect("exe parent");
    if cfg!(windows) { dir.join("ffmpeg.exe") } else { dir.join("ffmpeg") }
}

pub fn spawn(width: i32, height: i32, fps: u32, out_path: &str) -> Child {
    let ffmpeg = ffmpeg_path();
    let fps_str = fps.to_string();
    let size = format!("{width}x{height}");
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).to_string();
    let x265_params = format!(
        "log-level=1:alpha=1:pools={threads}:frame-threads={threads}:no-info=1"
    );
    let args: Vec<&str> = vec![
        "-loglevel", "error",
        "-y",
        "-f", "rawvideo",
        "-pixel_format", "bgra",
        "-video_size", &size,
        "-framerate", &fps_str,
        "-i", "-",
        // jellyfin-ffmpeg's libx265 ships with ENABLE_ALPHA — cross-platform bit-exact.
        "-c:v", "libx265",
        "-preset", "fast",
        "-crf", "23",
        "-pix_fmt", "yuva420p",
        "-tag:v", "hvc1",
        "-x265-params", &x265_params,
        "-movflags", "+faststart",
        out_path,
    ];
    let mut cmd = Command::new(ffmpeg);
    cmd.args(args)
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
