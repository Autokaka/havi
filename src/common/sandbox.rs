// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};

pub fn prepare_child(_cmd: &mut Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            _cmd.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
}

pub fn track_child(child: &Child) {
    register_ffmpeg(child.id());
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        crate::common::win_job::assign(child.as_raw_handle());
    }
}

fn pids() -> &'static Mutex<HashSet<i32>> {
    static PIDS: OnceLock<Mutex<HashSet<i32>>> = OnceLock::new();
    PIDS.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn register_ffmpeg(pid: u32) {
    if let Ok(pid) = i32::try_from(pid) {
        if pid > 0 { pids().lock().expect("pid set poisoned").insert(pid); }
    }
}

pub fn unregister_ffmpeg(pid: u32) {
    if let Ok(pid) = i32::try_from(pid) {
        pids().lock().expect("pid set poisoned").remove(&pid);
    }
}

fn kill_all_ffmpeg() {
    let drained: Vec<i32> = pids().lock().expect("pid set poisoned").drain().collect();
    #[cfg(unix)]
    for pid in drained {
        unsafe { libc::kill(pid, libc::SIGTERM); }
    }
    #[cfg(windows)]
    {
        let _ = drained;
        crate::common::win_job::terminate_all();
    }
}

pub fn sandbox_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let p = exe.parent().expect("exe parent").join("sandbox");
    let _ = std::fs::create_dir_all(&p);
    p
}

pub fn scratch_dir() -> &'static std::path::Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("havi-{}-{nanos:x}", std::process::id()));
        let _ = std::fs::create_dir_all(&p);
        p
    }).as_path()
}

pub fn cleanup_session() {
    kill_all_ffmpeg();
    let _ = std::fs::remove_dir_all(scratch_dir());
}

pub fn install_cleanup_hooks() {
    let _ = scratch_dir();
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        #[cfg(unix)]
        {
            use signal_hook::consts::*;
            let mut signals = signal_hook::iterator::Signals::new([SIGINT, SIGTERM, SIGHUP, SIGQUIT])
                .expect("install signals");
            std::thread::spawn(move || {
                if let Some(sig) = signals.forever().next() {
                    cleanup_session();
                    std::process::exit(128 + sig);
                }
            });
        }
        // panic=abort skips Drop; hook fires before abort.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            cleanup_session();
            prev(info);
        }));
    });
}
