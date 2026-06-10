// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::ipc::{Cmd, Evt, RenderId};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

#[cfg_attr(feature = "napi-binding", napi(object))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenderOpts {
    pub source: String,
    pub out: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub fps: Option<u32>,
    pub duration: Option<u32>,
    pub tolerant: Option<bool>,
    pub proxy: Option<Vec<crate::proxy::ProxyRule>>,
}

impl RenderOpts {
    pub fn out_or(&self) -> &str { self.out.as_deref().unwrap_or("out.mp4") }
    pub fn width_or(&self) -> i32 { self.width.unwrap_or(1920) }
    pub fn height_or(&self) -> i32 { self.height.unwrap_or(1080) }
    pub fn fps_or(&self) -> u32 { self.fps.unwrap_or(30) }
    pub fn duration_or(&self) -> u32 { self.duration.unwrap_or(5) }
}

pub struct HostClient {
    _child: Child,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<RenderId, Sender<Evt>>>>,
}

impl HostClient {
    pub fn spawn() -> std::io::Result<Arc<Self>> {
        let bin = locate_bin()?;
        let mut child = Command::new(bin)
            .arg("--host")
            .env(crate::ipc::ENV_FLAG, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdin = child.stdin.take().expect("host stdin");
        let stdout = child.stdout.take().expect("host stdout");
        let pending: Arc<Mutex<HashMap<RenderId, Sender<Evt>>>> = Arc::new(Mutex::new(HashMap::new()));

        {
            let pending = pending.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    let Ok(evt) = serde_json::from_str::<Evt>(&line) else { continue };
                    let id = match &evt {
                        Evt::Started { id } | Evt::Progress { id, .. }
                        | Evt::Console { id, .. } | Evt::Done { id, .. }
                        | Evt::Error { id, .. } => *id,
                        Evt::HostReady | Evt::HostExit => continue,
                    };
                    let tx = pending.lock().expect("pending poisoned").get(&id).cloned();
                    if let Some(tx) = tx { let _ = tx.send(evt); }
                }
                let mut map = pending.lock().expect("pending poisoned");
                for (id, tx) in map.drain() {
                    let _ = tx.send(Evt::Error { id, message: "host process exited".into() });
                }
            });
        }

        Ok(Arc::new(Self {
            _child: child,
            stdin: Mutex::new(stdin),
            next_id: AtomicU64::new(1),
            pending,
        }))
    }

    pub fn begin(&self, opts: RenderOpts) -> std::io::Result<(RenderId, Receiver<Evt>)> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.pending.lock().expect("pending poisoned").insert(id, tx);
        let cmd = Cmd::Start { id, opts };
        let line = serde_json::to_string(&cmd)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let mut w = self.stdin.lock().expect("stdin poisoned");
        writeln!(w, "{line}")?;
        w.flush()?;
        Ok((id, rx))
    }

    pub fn cancel(&self, id: RenderId) {
        let cmd = Cmd::Cancel { id };
        if let Ok(line) = serde_json::to_string(&cmd) {
            if let Ok(mut w) = self.stdin.lock() {
                let _ = writeln!(w, "{line}");
                let _ = w.flush();
            }
        }
    }

    pub fn forget(&self, id: RenderId) {
        self.pending.lock().expect("pending poisoned").remove(&id);
    }
}

fn locate_bin() -> std::io::Result<PathBuf> {
    let name = if cfg!(windows) { "havi.exe" } else { "havi" };
    if let Some(dir) = self_dylib_dir() {
        let p = dir.join(name);
        if p.exists() { return Ok(p); }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(name);
            if p.exists() { return Ok(p); }
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::NotFound, "havi binary not found alongside loader"))
}

#[cfg(unix)]
fn self_dylib_dir() -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_int, c_void};
    #[repr(C)]
    struct DlInfo {
        dli_fname: *const c_char,
        dli_fbase: *mut c_void,
        dli_sname: *const c_char,
        dli_saddr: *mut c_void,
    }
    extern "C" { fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int; }
    let mut info: DlInfo = unsafe { std::mem::zeroed() };
    let addr = self_dylib_dir as *const c_void;
    if unsafe { dladdr(addr, &mut info) } == 0 || info.dli_fname.is_null() { return None; }
    let path = unsafe { CStr::from_ptr(info.dli_fname) }.to_str().ok()?;
    PathBuf::from(path).parent().map(|p| p.to_path_buf())
}

#[cfg(windows)]
fn self_dylib_dir() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleExW(flags: u32, lp_module_name: *const std::os::raw::c_void, ph_module: *mut *mut std::os::raw::c_void) -> i32;
        fn GetModuleFileNameW(h_module: *mut std::os::raw::c_void, lp_filename: *mut u16, n_size: u32) -> u32;
    }
    const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x4;
    const GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT: u32 = 0x2;
    let mut module: *mut std::os::raw::c_void = std::ptr::null_mut();
    let addr = self_dylib_dir as *const std::os::raw::c_void;
    let ok = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            addr, &mut module,
        )
    };
    if ok == 0 { return None; }
    let mut buf = [0u16; 32768];
    let n = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 { return None; }
    let s = std::ffi::OsString::from_wide(&buf[..n as usize]);
    PathBuf::from(s).parent().map(|p| p.to_path_buf())
}
