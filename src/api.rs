// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::ipc::{self, Msg};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

#[cfg_attr(feature = "napi-binding", napi(object))]
#[derive(Debug, Clone)]
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
    pub fn out_or(&self) -> &str { self.out.as_deref().unwrap_or("out.mp4,out.webm") }
    pub fn width_or(&self) -> i32 { self.width.unwrap_or(1920) }
    pub fn height_or(&self) -> i32 { self.height.unwrap_or(1080) }
    pub fn fps_or(&self) -> u32 { self.fps.unwrap_or(30) }
    pub fn duration_or(&self) -> u32 { self.duration.unwrap_or(5) }
}

pub struct RenderHandle {
    child: Child,
    rx: mpsc::Receiver<Msg>,
}

impl RenderHandle {
    pub fn pid(&self) -> u32 { self.child.id() }
    pub fn try_recv(&mut self) -> Result<Msg, mpsc::TryRecvError> { self.rx.try_recv() }
    pub fn recv(&mut self) -> Option<Msg> { self.rx.recv().ok() }

    pub fn cancel(&mut self) {
        #[cfg(unix)]
        {
            let pid = i32::try_from(self.child.id()).unwrap_or(0);
            if pid > 0 { unsafe { libc::kill(pid, libc::SIGTERM); } }
        }
        #[cfg(windows)]
        { let _ = self.child.kill(); }
    }

    pub fn wait(mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait()
    }
}

pub fn spawn(opts: RenderOpts) -> std::io::Result<RenderHandle> {
    let bin = locate_bin()?;
    let mut cmd = Command::new(bin);
    cmd.arg(&opts.source)
        .args(["-W", &opts.width_or().to_string()])
        .args(["-H", &opts.height_or().to_string()])
        .args(["-f", &opts.fps_or().to_string()])
        .args(["-t", &opts.duration_or().to_string()])
        .args(["-o", opts.out_or()])
        .env(ipc::ENV_FLAG, "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if opts.tolerant.unwrap_or(false) { cmd.arg("--tolerant"); }
    if let Some(rules) = &opts.proxy {
        let json = serde_json::to_string(rules)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        cmd.arg("--proxy").arg(json);
    }
    let mut child = cmd.spawn()?;

    let stdout = child.stdout.take().expect("stdout piped");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Ok(msg) = serde_json::from_str::<Msg>(&line) {
                if tx.send(msg).is_err() { break; }
            }
        }
    });

    Ok(RenderHandle { child, rx })
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
