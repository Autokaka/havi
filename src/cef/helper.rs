//! Subprocess entry — renderer/GPU/utility. Registers the same App.

use havi_core::cef::app::{init_api, make_app};

fn main() {
    // Load libcef before init_api: api_hash lives in libcef; calling it
    // unloaded NULL-derefs and crashes the helper.
    #[cfg(target_os = "macos")]
    let _loader = {
        let exe = std::env::current_exe().expect("current_exe");
        let loader = cef::library_loader::LibraryLoader::new(&exe, true);
        assert!(loader.load(), "helper: failed to load Chromium Embedded Framework");
        loader
    };

    init_api();

    let args = cef::args::Args::new();
    let mut app = make_app();
    let code = cef::execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    std::process::exit(code);
}
