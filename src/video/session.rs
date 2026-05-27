// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::ipc;
use crate::video::decode;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub struct Session {
    pub id: String,
    pub frames_dir: PathBuf,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub ready: Arc<(Mutex<u32>, Condvar)>,
    pub done: Arc<Mutex<bool>>,
}

static FFMPEG: OnceLock<PathBuf> = OnceLock::new();
static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<Session>>>> = OnceLock::new();

pub fn set_ffmpeg(path: PathBuf) { let _ = FFMPEG.set(path); }

pub fn sessions() -> &'static Mutex<HashMap<String, Arc<Session>>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn id_for(src: &str, fps: u32) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    src.hash(&mut h);
    fps.hash(&mut h);
    format!("{:016x}", h.finish())
}

pub fn open(src: &str, fps: u32) -> Option<Arc<Session>> {
    let id = id_for(src, fps);
    if let Some(s) = sessions().lock().ok()?.get(&id).cloned() { return Some(s); }
    let ff = FFMPEG.get()?.clone();
    let (width, height, duration) = match decode::probe(&ff, src) {
        Some(v) => v,
        None => { ipc::error(&format!("ffprobe failed: {src}")); return None; }
    };
    let dir = crate::scratch_dir().join("frames").join(&id);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).ok()?;

    let ready = Arc::new((Mutex::new(0u32), Condvar::new()));
    let done = Arc::new(Mutex::new(false));
    let s = Arc::new(Session {
        id: id.clone(), frames_dir: dir.clone(), fps: fps as f64,
        width, height, duration, ready: ready.clone(), done: done.clone(),
    });
    sessions().lock().ok()?.insert(id, s.clone());

    let src_owned = src.to_string();
    std::thread::spawn(move || decode::run(ff, src_owned, dir, fps, ready, done));
    Some(s)
}

pub fn wait_frame(s: &Session, idx: u32, timeout: Duration) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    loop {
        let p = s.frames_dir.join(format!("{idx:06}.png"));
        if let Ok(b) = std::fs::read(&p) { return Some(b); }
        let g = s.ready.0.lock().ok()?;
        if *g >= idx { continue; }
        if *s.done.lock().ok()? { return None; }
        let now = Instant::now();
        if now >= deadline { return None; }
        let _ = s.ready.1.wait_timeout(g, deadline - now).ok()?;
    }
}

pub fn close(id: &str) {
    if let Some(s) = sessions().lock().ok().and_then(|mut m| m.remove(id)) {
        let _ = std::fs::remove_dir_all(&s.frames_dir);
    }
}
