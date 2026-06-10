// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

use crate::api::RenderOpts;
use crate::cef::cdp::{Cdp, CdpObserver};
use crate::cef::client::DetClient;
use crate::host::ipc::emit_evt;
use crate::host::render::{CaptureState, Host, Render};
use crate::ipc::{Evt, RenderId};
use crate::renderer::capture::{install_budget_listener, BrowserHandle, FrameHandle};
use crate::renderer::host::write_host;
use crate::renderer::load::{DetLoadHandler, TolerantTimeoutTask, LOAD_TIMEOUT_MS};
use crate::renderer::paint::CaptureHandler;
use crate::video::encoder;
use cef::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub fn max_parallel() -> usize {
    std::env::var("HAVI_MAX_PARALLEL").ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(4)
}

pub fn install_start_fn(host: &Arc<Host>) {
    host.set_start_fn(Box::new(|host, id, opts| start_render(host, id, opts)));
}

fn start_render(host: &Arc<Host>, id: RenderId, opts: RenderOpts) {
    let width = opts.width_or();
    let height = opts.height_or();
    let fps = opts.fps_or();
    let duration = opts.duration_or();
    let total_frames = fps * duration;
    let frame_ms = 1000.0 / f64::from(fps);
    let out = opts.out_or().to_string();

    let enc = encoder::start(width, height, fps, &out);
    let tx = enc.tx.clone();

    let browser: BrowserHandle = Arc::new(Mutex::new(None));
    let iframe: FrameHandle = Arc::new(Mutex::new(None));
    let cdp = Cdp::new();

    let render = Arc::new(Mutex::new(Render {
        id, width, height, total_frames, frame_ms,
        out: out.clone(), tolerant: opts.tolerant.unwrap_or(false),
        phase: 0, browser: browser.clone(), iframe,
        cdp: cdp.clone(),
        capture: CaptureState { next_frame: 0, requested_ms: 0, budget_done: false, stuck_invalidates: 0 },
        encoder: Some(enc), tx: Some(tx), done: false, errored: false,
        devtools: None, started_at: Instant::now(),
    }));
    host.insert(render.clone());
    install_budget_listener(&render);

    let proxy = opts.proxy.as_ref().map(|rules| {
        let json = serde_json::to_string(rules).unwrap_or_default();
        Arc::new(crate::proxy::Compiled::from_json(&json).expect("invalid proxy"))
    });

    let render_handler = CaptureHandler::new(host.clone());
    let load_handler = DetLoadHandler::new(host.clone());
    let mut client = DetClient::new(render_handler, load_handler, proxy);

    let w = u32::try_from(width).expect("width");
    let h = u32::try_from(height).expect("height");
    let host_url = write_host(&opts.source, w, h).expect("write host page");

    let window_info = WindowInfo::default().set_as_windowless(Default::default());
    let mut browser_settings = BrowserSettings::default();
    browser_settings.windowless_frame_rate = 240;
    browser_settings.background_color = 0;
    let b = browser_host_create_browser_sync(
        Some(&window_info),
        Some(&mut client),
        Some(&CefString::from(host_url.as_str())),
        Some(&browser_settings),
        None, None,
    ).expect("browser_host_create_browser_sync failed");

    let browser_id = b.identifier();
    let reg = b.host().and_then(|bh| {
        bh.was_resized();
        let mut obs = CdpObserver::new(cdp.clone());
        bh.add_dev_tools_message_observer(Some(&mut obs))
    });
    render.lock().expect("render poisoned").devtools = reg;
    *browser.lock().expect("browser poisoned") = Some(b);
    host.bind_browser(browser_id, id);

    emit_evt(&Evt::Started { id });

    let mut tt = TolerantTimeoutTask::new(host.clone(), id);
    post_delayed_task(ThreadId::UI, Some(&mut tt), LOAD_TIMEOUT_MS);
}
