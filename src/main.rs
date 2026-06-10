// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use cef::*;
use clap::Parser;
use havi_core::api::RenderOpts;
use havi_core::cef::app::{init_api, make_app};
use havi_core::cli::Cli;
use havi_core::host::render::Host;
use havi_core::host::run::{install_start_fn, max_parallel, run};
use havi_core::video::encoder;
use havi_core::video::scheme::{self, HaviFrameFactory, SCHEME};

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

    havi_core::install_cleanup_hooks();
    havi_core::install_parent_death_watcher();
    cef_init(&cef_args, &mut app);

    scheme::set_ffmpeg(encoder::ffmpeg_path());
    let mut factory = HaviFrameFactory::new();
    register_scheme_handler_factory(Some(&CefString::from(SCHEME)), None, Some(&mut factory));

    let host = Host::new(max_parallel());

    {
        let host = host.clone();
        scheme::set_on_dom_ready(move || host.dom_ready_advance());
    }

    if !cli.host {
        let source = match cli.source.clone() {
            Some(s) => s,
            None => {
                eprintln!("error: source required (or pass --host)");
                std::process::exit(2);
            }
        };
        let opts = RenderOpts {
            source,
            out: Some(cli.out.clone()),
            width: Some(cli.width),
            height: Some(cli.height),
            fps: Some(cli.fps),
            duration: Some(cli.duration),
            tolerant: Some(cli.tolerant),
            proxy: cli.proxy.as_ref().map(|s| serde_json::from_str(s).expect("invalid --proxy JSON")),
        };
        install_start_fn(&host);
        eprintln!("rendering {} → {}", opts.source, cli.out);
        host.set_single_shot(0);
        host.submit(0, opts);
        run_message_loop();
        shutdown();
        havi_core::cleanup_session();
        return;
    }

    run(host);
    shutdown();
    havi_core::cleanup_session();
}

fn cef_init(args: &cef::args::Args, app: &mut App) {
    let mut settings = Settings::default();
    settings.no_sandbox = 1;
    settings.windowless_rendering_enabled = 1;
    settings.log_severity = LogSeverity::DISABLE;
    let profile_dir = havi_core::sandbox_dir().join("profile");
    let _ = std::fs::create_dir_all(&profile_dir);
    for f in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
        let _ = std::fs::remove_file(profile_dir.join(f));
    }
    settings.root_cache_path = CefString::from(profile_dir.to_string_lossy().as_ref());
    settings.cache_path = CefString::from(profile_dir.to_string_lossy().as_ref());
    assert_eq!(
        initialize(Some(args.as_main_args()), Some(&settings), Some(app), std::ptr::null_mut()),
        1, "cef initialize failed"
    );
}
