// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use cef::*;
use clap::Parser;
use havi_core::cef::app::{init_api, make_app};
use havi_core::cef::cdp::{Cdp, CdpObserver};
use havi_core::cef::client::DetClient;
use havi_core::cli::Cli;
use havi_core::renderer::capture::{install_budget_listener, BrowserHandle, Shared, State};
use havi_core::renderer::host::write_host;
use havi_core::renderer::load::{advance_phase, DetLoadHandler, TolerantTimeoutTask, LOAD_TIMEOUT_MS};
use havi_core::renderer::paint::CaptureHandler;
use havi_core::video::encoder;
use havi_core::video::scheme::{self, HaviFrameFactory, SCHEME};
use std::sync::{Arc, Mutex};

fn main() {
    #[cfg(target_os = "macos")]
    let _loader = {
        let exe = std::env::current_exe().expect("current_exe");
        let loader = cef::library_loader::LibraryLoader::new(&exe, false);
        assert!(loader.load(), "failed to load Chromium Embedded Framework");
        loader
    };

    init_api();
    let cef_args = cef::args::Args::new();
    let mut app = make_app();
    let code = execute_process(Some(cef_args.as_main_args()), Some(&mut app), std::ptr::null_mut());
    if code >= 0 { std::process::exit(code); }

    let cli = Cli::parse();
    let total_frames = cli.fps * cli.duration;
    let frame_ms = 1000.0 / f64::from(cli.fps);

    havi_core::install_cleanup_hooks();
    havi_core::install_parent_death_watcher();
    cef_init(&cef_args, &mut app);

    scheme::set_ffmpeg(encoder::ffmpeg_path());
    let mut factory = HaviFrameFactory::new();
    register_scheme_handler_factory(Some(&CefString::from(SCHEME)), None, Some(&mut factory));

    let outs: Vec<String> = cli.out.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let (mut ffmpeg, ff_err_tail) = encoder::spawn(cli.width, cli.height, cli.fps, &outs);
    havi_core::register_ffmpeg(ffmpeg.id());
    let (tx, encoder_thread) = encoder::start_pipe(&mut ffmpeg);

    let browser_handle: BrowserHandle = Arc::new(Mutex::new(None));
    let cdp = Cdp::new();
    let state: Shared = Arc::new(Mutex::new(State {
        next_frame: 0,
        requested_ms: 0,
        budget_done: false,
        stuck_invalidates: 0,
        tx: Some(tx),
        done: false,
        reload_fired: false,
        browser: browser_handle.clone(),
        iframe: Arc::new(Mutex::new(None)),
        cdp: cdp.clone(),
        width: cli.width,
        height: cli.height,
        total_frames,
        frame_ms,
    }));
    install_budget_listener(state.clone());

    let render = CaptureHandler::new(state.clone());
    let phase = Arc::new(Mutex::new(0u8));
    let load = DetLoadHandler::new(state.clone(), phase.clone(), cli.tolerant);
    if cli.tolerant {
        let st = state.clone();
        let ph = phase.clone();
        scheme::set_on_dom_ready(move || advance_phase(&st, &ph, None));
    }
    let proxy = cli.proxy.as_deref().map(|s| {
        Arc::new(havi_core::proxy::Compiled::from_json(s).expect("invalid --proxy JSON"))
    });
    let mut client = DetClient::new(render, load, proxy);

    let w = u32::try_from(cli.width).expect("width must be non-negative");
    let h = u32::try_from(cli.height).expect("height must be non-negative");
    let host_url = write_host(&cli.source, w, h).expect("write host page");
    eprintln!("rendering {} → {}", cli.source, cli.out);
    let started = std::time::Instant::now();

    let browser = create_browser(&mut client, &host_url);
    let _devtools = browser.host().and_then(|host| {
        host.was_resized();
        let mut obs = CdpObserver::new(cdp.clone());
        host.add_dev_tools_message_observer(Some(&mut obs))
    });
    *browser_handle.lock().expect("browser handle poisoned") = Some(browser);

    let mut timeout_task = TolerantTimeoutTask::new(state.clone(), phase.clone(), cli.tolerant);
    post_delayed_task(ThreadId::UI, Some(&mut timeout_task), LOAD_TIMEOUT_MS);

    run_message_loop();

    state.lock().expect("state poisoned").tx = None;
    encoder_thread.join().expect("encoder thread");
    let ffmpeg_pid = ffmpeg.id();
    let status = ffmpeg.wait().expect("ffmpeg wait failed");
    havi_core::unregister_ffmpeg(ffmpeg_pid);
    if !status.success() {
        let tail = ff_err_tail.join().unwrap_or_default().join(" | ");
        havi_core::ipc::error(&format!("ffmpeg encoder exited with {status}: {tail}"));
        havi_core::cleanup_session();
        shutdown();
        std::process::exit(1);
    }

    shutdown();
    havi_core::cleanup_session();
    report_done(&cli, total_frames, started.elapsed());
}

fn cef_init(args: &cef::args::Args, app: &mut App) {
    let mut settings = Settings::default();
    settings.no_sandbox = 1;
    settings.windowless_rendering_enabled = 1;
    settings.log_severity = LogSeverity::DISABLE;
    // Isolated per-process cache — unique path means own SingletonLock, no
    // contention when many havi processes run at once. Removed on exit.
    let cache_dir = havi_core::scratch_dir().join("cache");
    let _ = std::fs::create_dir_all(&cache_dir);
    settings.root_cache_path = CefString::from(cache_dir.to_string_lossy().as_ref());
    assert_eq!(
        initialize(Some(args.as_main_args()), Some(&settings), Some(app), std::ptr::null_mut()),
        1, "cef initialize failed"
    );
}

fn create_browser(client: &mut Client, host_url: &str) -> Browser {
    let window_info = WindowInfo::default().set_as_windowless(Default::default());
    let mut browser_settings = BrowserSettings::default();
    browser_settings.windowless_frame_rate = 240;
    // Transparent OSR — captured BGRA carries real alpha. ARGB 0 = fully transparent.
    browser_settings.background_color = 0;
    browser_host_create_browser_sync(
        Some(&window_info),
        Some(client),
        Some(&CefString::from(host_url)),
        Some(&browser_settings),
        None, None,
    ).expect("browser_host_create_browser_sync failed")
}

fn report_done(cli: &Cli, total_frames: u32, elapsed: std::time::Duration) {
    let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    if havi_core::ipc::enabled() {
        havi_core::ipc::emit(&havi_core::ipc::Msg::Done {
            frames: total_frames, width: cli.width, height: cli.height,
            fps: cli.fps, out: cli.out.clone(), elapsed_ms,
        });
    } else {
        println!(
            "done: {} ({total_frames} frames, {}x{} @ {}fps) in {:.2}s",
            cli.out, cli.width, cli.height, cli.fps,
            elapsed_ms as f64 / 1000.0,
        );
    }
}
