pub fn deterministic_flags() -> Vec<String> {
    let mut flags = vec![
        "use-mock-keychain".to_string(),
        "enable-begin-frame-control".to_string(),
        "disable-threaded-animation".to_string(),
        "disable-threaded-scrolling".to_string(),
        "disable-checker-imaging".to_string(),
        "disable-image-animation-resync".to_string(),
        "disable-features=PaintHolding".to_string(),
        "force-device-scale-factor=1".to_string(),
        "force-color-profile=srgb".to_string(),
        "font-render-hinting=none".to_string(),
        // Host reaches into iframe.contentWindow + havi-frame:// fetched cross-origin.
        "disable-web-security".to_string(),
        "disable-site-isolation-trials".to_string(),
        "allow-file-access-from-files".to_string(),
        "ignore-certificate-errors".to_string(),
        "disable-dev-shm-usage".to_string(),
        // Auto-deny everything a headless renderer never needs. Prevents
        // runtime permission prompts and macOS TCC requests (camera/mic/etc).
        "deny-permission-prompts".to_string(),
        "disable-notifications".to_string(),
        "mute-audio".to_string(),
        "autoplay-policy=no-user-gesture-required".to_string(),
        "disable-background-networking".to_string(),
        "disable-background-timer-throttling".to_string(),
        "disable-renderer-backgrounding".to_string(),
        "disable-backgrounding-occluded-windows".to_string(),
        "disable-extensions".to_string(),
        "disable-default-apps".to_string(),
        "disable-popup-blocking".to_string(),
        "disable-prompt-on-repost".to_string(),
        "disable-sync".to_string(),
        "disable-blink-features=AutomationControlled".to_string(),
        "no-default-browser-check".to_string(),
        "no-first-run".to_string(),
        "ignore-gpu-blocklist".to_string(),
        "disable-gpu-sandbox".to_string(),
    ];

    #[cfg(target_os = "macos")]
    flags.push("use-angle=metal".to_string());
    #[cfg(target_os = "windows")]
    flags.push("use-angle=d3d11".to_string());
    #[cfg(target_os = "linux")]
    {
        flags.push("use-angle=vulkan".to_string());
        flags.push("disable-vulkan-surface".to_string());
        flags.push("enable-features=Vulkan".to_string());
    }

    flags
}
