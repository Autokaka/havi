# Multi-render Host Process Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor `havi` from "one subprocess per render" to one persistent host process that owns CEF and runs N concurrent renders, all sharing one `RequestContext` (HTTP cache + cookies + GPU shader cache), eliminating SingletonLock contention.

**Architecture:** A single `havi --host` process initializes CEF once and accepts line-delimited JSON commands (`start`/`cancel`/`shutdown`) on stdin, emitting tagged events on stdout. Each render is a `Render` struct keyed by a caller-assigned `RenderId`; CEF callbacks route to the right render via `browser.identifier()`. The napi binding lazily spawns one host per Node process and fans render-events out to per-call promises. The CLI is a thin wrapper that drives the host with a single render.

**Tech Stack:** Rust + cef-rs 148.2.0, napi-rs 3.x, serde/serde_json for the wire format, ffmpeg (jellyfin-ffmpeg) as encoder sidecar, Bun for build orchestration.

---

## File Structure

New files:
- `src/host/mod.rs` — `Host` container, `Render` per-render state, `RenderId`, lifecycle (start/cancel/finish/queue).
- `src/host/ipc.rs` — IPC v2 `Cmd`/`Evt` enums (one wire format, two consumers), stdin reader + stdout emitter.
- `src/host/run.rs` — host bootstrap: CEF init, RequestContext creation, message loop, command dispatch.

Modified files:
- `src/renderer/capture.rs` — `State` → `CaptureState` (no `Shared` global Arc<Mutex>); per-render handle drawn from `Render`.
- `src/renderer/paint.rs` — `CaptureHandler` resolves render by browser id; uses per-render emit for progress.
- `src/renderer/load.rs` — `DetLoadHandler` resolves render by browser id; per-render phase.
- `src/cef/cdp.rs` — `CdpObserver` carries `RenderId`; events route to that render.
- `src/common/ipc.rs` — add `RenderId`-tagged `Evt`/`Cmd`; keep human-mode progress bar for CLI.
- `src/cli.rs` — add `--host` flag.
- `src/main.rs` — shrink to: CEF subprocess-bootstrap, then `host::run::main()`.
- `src/napi/mod.rs` — `OnceLock<HostHandle>`, reader-thread fan-out, per-call promise.
- `src/api.rs` — `RenderOpts` stays; `spawn`/`RenderHandle` repurposed into host client (or deleted in favor of host IPC).
- `src/lib.rs` — register `pub mod host`.
- `havi/index.d.ts` — unchanged surface, doc note about parallelism.

---

## Conventions for every task

- Build check after Rust edits: `cargo build --bin havi 2>&1 | tail -20` (host binary). For napi: `cargo build --lib --features napi-binding 2>&1 | tail -20`.
- This repo has **no unit-test harness today** (no `tests/` dir, no `#[cfg(test)]` modules). We add `#[cfg(test)]` modules where logic is pure (IPC serde, routing). CEF-dependent behavior is verified by smoke runs against `stamp.html`, not unit tests — CEF cannot init twice in one test process.
- STRIKE 7 comment rule: no comments unless one-line WHY for a non-obvious constraint.
- Commit after each task with the message shown.

---

## Task 1: IPC v2 wire format (`Cmd` / `Evt`)

**Files:**
- Modify: `src/common/ipc.rs`
- Test: inline `#[cfg(test)]` in `src/common/ipc.rs`

The current `Msg` enum has no render id. Multi-render needs every event tagged with which render it belongs to. Add a new `RenderId` alias plus `Cmd` (parent→host) and `Evt` (host→parent) enums. Keep the existing `Msg`/`emit`/`console`/`error` functions for now — CLI human-mode still uses them; we migrate callers in later tasks.

- [ ] **Step 1: Write the failing test**

Add to the end of `src/common/ipc.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_roundtrip_start() {
        let opts = crate::api::RenderOpts {
            source: "a.html".into(),
            out: Some("o.mp4".into()),
            width: Some(800), height: Some(600),
            fps: Some(30), duration: Some(5),
            tolerant: Some(false), proxy: None,
        };
        let cmd = Cmd::Start { id: 7, opts };
        let line = serde_json::to_string(&cmd).unwrap();
        assert!(line.contains(r#""cmd":"start""#));
        assert!(line.contains(r#""id":7"#));
        let back: Cmd = serde_json::from_str(&line).unwrap();
        matches!(back, Cmd::Start { id: 7, .. });
    }

    #[test]
    fn evt_roundtrip_done() {
        let evt = Evt::Done { id: 3, out: "o.mp4".into(), frames: 150, elapsed_ms: 4200 };
        let line = serde_json::to_string(&evt).unwrap();
        assert!(line.contains(r#""evt":"done""#));
        let back: Evt = serde_json::from_str(&line).unwrap();
        if let Evt::Done { id, frames, .. } = back {
            assert_eq!(id, 3);
            assert_eq!(frames, 150);
        } else { panic!("wrong variant"); }
    }

    #[test]
    fn evt_host_ready_tagless() {
        let line = serde_json::to_string(&Evt::HostReady).unwrap();
        assert_eq!(line, r#"{"evt":"host_ready"}"#);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib ipc:: 2>&1 | tail -20`
Expected: FAIL — `Cmd` / `Evt` / `RenderId` not defined.

- [ ] **Step 3: Add the types**

In `src/common/ipc.rs`, after the existing `Msg` enum (around line 52), add:

```rust
pub type RenderId = u64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Cmd {
    Start { id: RenderId, opts: crate::api::RenderOpts },
    Cancel { id: RenderId },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "evt", rename_all = "snake_case")]
pub enum Evt {
    HostReady,
    Started { id: RenderId },
    Progress { id: RenderId, frame: u32, total: u32 },
    Console { id: RenderId, level: Level, source: String, message: String },
    Done { id: RenderId, out: String, frames: u32, elapsed_ms: u64 },
    Error { id: RenderId, message: String },
    HostExit,
}
```

`RenderOpts` must be `Serialize + Deserialize`. It currently derives only `Debug, Clone`. Add the derives in `src/api.rs`:

Change line 9-10 of `src/api.rs`:

```rust
#[cfg_attr(feature = "napi-binding", napi(object))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenderOpts {
```

`ProxyRule` (in `src/proxy.rs`) is already `Serialize` (used by `--proxy` JSON emit in `api::spawn`). Verify it also derives `Deserialize`; if not, add it.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib ipc:: 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/common/ipc.rs src/api.rs src/proxy.rs
git commit -m "feat(ipc): add render-id-tagged Cmd/Evt wire format"
```

---

## Task 2: `Evt` stdout emitter + `Cmd` stdin reader

**Files:**
- Create: `src/host/mod.rs` (module stub)
- Create: `src/host/ipc.rs`
- Modify: `src/lib.rs`
- Test: inline `#[cfg(test)]` in `src/host/ipc.rs`

Host writes `Evt` JSON lines to stdout (one per line, flushed). Parent writes `Cmd` JSON lines to host stdin. This task builds the thin serialize/parse + emit layer. No CEF yet.

- [ ] **Step 1: Register module in lib.rs**

In `src/lib.rs`, after `pub mod cli;` (line 9), add:

```rust
pub mod host;
```

- [ ] **Step 2: Create module stub**

Create `src/host/mod.rs`:

```rust
// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

pub mod ipc;
```

- [ ] **Step 3: Write the failing test**

Create `src/host/ipc.rs`:

```rust
// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

use crate::ipc::{Cmd, Evt};
use std::io::Write;

pub fn emit_evt(evt: &Evt) {
    let Ok(line) = serde_json::to_string(evt) else { return };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

pub fn parse_cmd(line: &str) -> Option<Cmd> {
    let line = line.trim();
    if line.is_empty() { return None; }
    serde_json::from_str(line).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_start_line() {
        let line = r#"{"cmd":"start","id":1,"opts":{"source":"a.html"}}"#;
        let cmd = parse_cmd(line).expect("parse");
        matches!(cmd, Cmd::Start { id: 1, .. });
    }

    #[test]
    fn parse_blank_is_none() {
        assert!(parse_cmd("   ").is_none());
    }

    #[test]
    fn parse_garbage_is_none() {
        assert!(parse_cmd("not json").is_none());
    }
}
```

- [ ] **Step 4: Run test to verify it passes (and compiles)**

Run: `cargo test --lib host::ipc 2>&1 | tail -20`
Expected: PASS (3 tests). The `RenderOpts` `Deserialize` from Task 1 lets `{"source":"a.html"}` parse with all-optional fields defaulting to `None`.

Note: `RenderOpts.source` is non-Option, so `{"source":"a.html"}` is the minimal valid opts. Confirm the test line includes `source`.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/host/mod.rs src/host/ipc.rs
git commit -m "feat(host): Evt stdout emitter + Cmd stdin parser"
```

---

## Task 3: `Render` + `Host` state containers

**Files:**
- Create: `src/host/render.rs`
- Modify: `src/host/mod.rs`
- Test: inline `#[cfg(test)]` in `src/host/render.rs`

Lift per-render state into a `Render` struct. `Host` holds the render registry keyed by `RenderId` plus a `browser-id → RenderId` map for routing CEF callbacks. This task defines the structs and the registry operations (insert, lookup-by-browser, remove). No CEF wiring yet — pure data structures + a routing test.

- [ ] **Step 1: Write the failing test**

Create `src/host/render.rs`:

```rust
// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

use crate::cef::cdp::Cdp;
use crate::ipc::RenderId;
use crate::renderer::capture::{BrowserHandle, FrameHandle};
use std::collections::HashMap;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub struct CaptureState {
    pub next_frame: u32,
    pub requested_ms: u32,
    pub budget_done: bool,
    pub stuck_invalidates: u32,
}

pub struct Render {
    pub id: RenderId,
    pub width: i32,
    pub height: i32,
    pub total_frames: u32,
    pub frame_ms: f64,
    pub out: String,
    pub tolerant: bool,
    pub phase: u8,
    pub browser: BrowserHandle,
    pub iframe: FrameHandle,
    pub cdp: Cdp,
    pub capture: CaptureState,
    pub encoder_pid: Option<u32>,
    pub tx: Option<SyncSender<Vec<u8>>>,
    pub done: bool,
    pub started_at: Instant,
}

pub type RenderRef = Arc<Mutex<Render>>;

#[derive(Default)]
pub struct Host {
    renders: Mutex<HashMap<RenderId, RenderRef>>,
    by_browser: Mutex<HashMap<i32, RenderId>>,
}

impl Host {
    pub fn new() -> Self { Self::default() }

    pub fn insert(&self, render: RenderRef) {
        let id = render.lock().expect("render poisoned").id;
        self.renders.lock().expect("renders poisoned").insert(id, render);
    }

    pub fn bind_browser(&self, browser_id: i32, render_id: RenderId) {
        self.by_browser.lock().expect("by_browser poisoned").insert(browser_id, render_id);
    }

    pub fn by_id(&self, id: RenderId) -> Option<RenderRef> {
        self.renders.lock().expect("renders poisoned").get(&id).cloned()
    }

    pub fn by_browser(&self, browser_id: i32) -> Option<RenderRef> {
        let id = *self.by_browser.lock().expect("by_browser poisoned").get(&browser_id)?;
        self.by_id(id)
    }

    pub fn remove(&self, id: RenderId) -> Option<RenderRef> {
        self.by_browser.lock().expect("by_browser poisoned").retain(|_, v| *v != id);
        self.renders.lock().expect("renders poisoned").remove(&id)
    }

    pub fn active_count(&self) -> usize {
        self.renders.lock().expect("renders poisoned").len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_render(id: RenderId) -> RenderRef {
        Arc::new(Mutex::new(Render {
            id, width: 16, height: 16, total_frames: 1, frame_ms: 33.0,
            out: "o.mp4".into(), tolerant: false, phase: 0,
            browser: Arc::new(Mutex::new(None)),
            iframe: Arc::new(Mutex::new(None)),
            cdp: Cdp::new(),
            capture: CaptureState { next_frame: 0, requested_ms: 0, budget_done: false, stuck_invalidates: 0 },
            encoder_pid: None, tx: None, done: false, started_at: Instant::now(),
        }))
    }

    #[test]
    fn route_by_browser_id() {
        let host = Host::new();
        host.insert(fake_render(1));
        host.insert(fake_render(2));
        host.bind_browser(101, 1);
        host.bind_browser(102, 2);
        assert_eq!(host.by_browser(101).unwrap().lock().unwrap().id, 1);
        assert_eq!(host.by_browser(102).unwrap().lock().unwrap().id, 2);
        assert!(host.by_browser(999).is_none());
    }

    #[test]
    fn remove_clears_browser_binding() {
        let host = Host::new();
        host.insert(fake_render(1));
        host.bind_browser(101, 1);
        assert_eq!(host.active_count(), 1);
        host.remove(1);
        assert_eq!(host.active_count(), 0);
        assert!(host.by_browser(101).is_none());
    }
}
```

- [ ] **Step 2: Register module**

In `src/host/mod.rs`:

```rust
// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

pub mod ipc;
pub mod render;
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test --lib host::render 2>&1 | tail -20`
Expected: PASS (2 tests).

If `Cdp::new()` is private or missing a `Default`, confirm it's `pub fn new()` (it is, per `src/cef/cdp.rs:20`).

- [ ] **Step 4: Commit**

```bash
git add src/host/mod.rs src/host/render.rs
git commit -m "feat(host): Render + Host registry with browser-id routing"
```

---

## Task 4: Per-render CDP routing

**Files:**
- Modify: `src/cef/cdp.rs`

Today `CdpObserver` holds one `Cdp` and fans events to all handlers registered on it. With N browsers, each browser's DevTools observer must route only to its own render's `Cdp`. Each `Render` already owns a distinct `Cdp` (Task 3). The observer just needs to wrap that render's `Cdp` — which it already does structurally. The only change: confirm `on_dev_tools_event` dispatches to the observer's own `cdp`, which is per-render. No code change to dispatch logic needed; the isolation comes from each browser getting its own `CdpObserver::new(render.cdp.clone())`.

This task is a **verification + doc** task: add a test proving two independent `Cdp` instances don't cross-talk.

- [ ] **Step 1: Write the failing test**

Add to the end of `src/cef/cdp.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn handlers_isolated_per_cdp() {
        let a = Cdp::new();
        let b = Cdp::new();
        let a_hits = Arc::new(AtomicU32::new(0));
        let b_hits = Arc::new(AtomicU32::new(0));
        let ac = a_hits.clone();
        let bc = b_hits.clone();
        a.on_event("Foo.bar", move |_| { ac.fetch_add(1, Ordering::SeqCst); });
        b.on_event("Foo.bar", move |_| { bc.fetch_add(1, Ordering::SeqCst); });

        // Fire only A's handlers.
        for (m, h) in a.0.event_handlers.lock().unwrap().iter() {
            if m == "Foo.bar" { h(&[]); }
        }
        assert_eq!(a_hits.load(Ordering::SeqCst), 1);
        assert_eq!(b_hits.load(Ordering::SeqCst), 0);
    }
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test --lib cdp::tests 2>&1 | tail -20`
Expected: PASS — `event_handlers` is `pub` (it's accessed as `self.cdp.0.event_handlers` in the observer), so the test can reach it.

If `event_handlers` is not reachable from the test (private field), make `CdpInner` fields `pub` (they already are accessed via `.0.event_handlers` in `on_dev_tools_event`, so confirm `pub`).

- [ ] **Step 3: Commit**

```bash
git add src/cef/cdp.rs
git commit -m "test(cdp): prove per-instance handler isolation"
```

---

## Task 5: Migrate `capture.rs` to per-render state

**Files:**
- Modify: `src/renderer/capture.rs`

Replace the `State`/`Shared` global model with functions that operate on `RenderRef`. The step/advance/stego logic is unchanged; only the state container changes from `Arc<Mutex<State>>` to `Arc<Mutex<Render>>` (`RenderRef`).

- [ ] **Step 1: Rewrite capture.rs**

Replace the entire contents of `src/renderer/capture.rs` with:

```rust
// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::cef::cdp::Cdp;
use crate::host::render::RenderRef;
use cef::*;
use std::sync::{Arc, Mutex};

pub type BrowserHandle = Arc<Mutex<Option<Browser>>>;
pub type FrameHandle = Arc<Mutex<Option<Frame>>>;

pub fn pause_virtual_time(cdp: &Cdp, host: &BrowserHost) {
    cdp.send(host, "Emulation.setVirtualTimePolicy", r#"{"policy":"pause"}"#);
}

fn advance_virtual_time(cdp: &Cdp, host: &BrowserHost, budget_ms: u32) {
    let params = format!(
        r#"{{"policy":"advance","budget":{budget_ms},"maxVirtualTimeTaskStarvationCount":1}}"#
    );
    cdp.send(host, "Emulation.setVirtualTimePolicy", &params);
}

fn draw_stego(cdp: &Cdp, host: &BrowserHost, ms: u32) {
    let params = format!(r#"{{"expression":"window.__havi_step({ms})","returnByValue":true}}"#);
    cdp.send(host, "Runtime.evaluate", &params);
}

fn tick_iframe(frame: &Frame, ms: u32) {
    let code = format!(
        "(function(){{if(window.__havi_tick)window.__havi_tick.process({ms});if(window.__havi_advance_videos)window.__havi_advance_videos({ms});}})()"
    );
    frame.execute_java_script(Some(&CefString::from(code.as_str())), None, 0);
}

pub fn step_frame(render: &RenderRef) {
    let (cdp, browser, iframe, budget_ms, ms) = {
        let r = render.lock().expect("render poisoned");
        if r.done { return; }
        let ms = (((r.capture.next_frame + 1) as f64) * r.frame_ms).floor() as u32;
        (r.cdp.clone(), r.browser.clone(), r.iframe.clone(), r.frame_ms as u32, ms)
    };

    let Some(host) = browser.lock().expect("browser poisoned").as_ref().and_then(|b| b.host()) else {
        return;
    };

    {
        let mut r = render.lock().expect("render poisoned");
        r.capture.requested_ms = ms;
        r.capture.budget_done = false;
        r.capture.stuck_invalidates = 0;
    }

    draw_stego(&cdp, &host, ms);
    if let Some(f) = iframe.lock().expect("iframe poisoned").as_ref() {
        tick_iframe(f, ms);
    }
    advance_virtual_time(&cdp, &host, budget_ms);
}

wrap_task! {
    pub struct StepTask { pub render: RenderRef }
    impl Task {
        fn execute(&self) { step_frame(&self.render); }
    }
}

pub fn schedule_step(render: &RenderRef) {
    let mut task = StepTask::new(render.clone());
    post_task(ThreadId::UI, Some(&mut task));
}

wrap_task! {
    pub struct InvalidateTask { pub browser: BrowserHandle }
    impl Task {
        fn execute(&self) {
            let host = self.browser.lock().expect("browser poisoned").as_ref().and_then(|b| b.host());
            if let Some(h) = host { h.invalidate(PaintElementType::VIEW); }
        }
    }
}

pub fn schedule_invalidate(browser: BrowserHandle) {
    let mut task = InvalidateTask::new(browser);
    post_task(ThreadId::UI, Some(&mut task));
}

pub fn install_budget_listener(render: &RenderRef) {
    let (cdp, weak) = {
        let r = render.lock().expect("render poisoned");
        (r.cdp.clone(), Arc::downgrade(render))
    };
    cdp.on_event("Emulation.virtualTimeBudgetExpired", move |_| {
        let Some(render) = weak.upgrade() else { return };
        let browser = {
            let mut r = render.lock().expect("render poisoned");
            r.capture.budget_done = true;
            r.browser.clone()
        };
        schedule_invalidate(browser);
    });
}
```

Note: `install_budget_listener` now takes `&RenderRef` and holds a `Weak` so a finished/removed render's listener becomes a no-op instead of resurrecting it.

- [ ] **Step 2: Build check (expect errors in paint.rs/load.rs/main.rs)**

Run: `cargo build --bin havi 2>&1 | grep -E "error\[|error:" | head -20`
Expected: errors only in `paint.rs`, `load.rs`, `main.rs` (they still reference old `State`/`Shared`). `capture.rs` itself compiles. We fix consumers in Tasks 6-8.

- [ ] **Step 3: Commit**

```bash
git add src/renderer/capture.rs
git commit -m "refactor(capture): operate on RenderRef instead of global Shared"
```

---

## Task 6: Migrate `paint.rs` to per-render routing

**Files:**
- Modify: `src/renderer/paint.rs`

`CaptureHandler` no longer holds a single `Shared`. It holds `Arc<Host>`, resolves the render via `browser.identifier()`, and emits progress as an `Evt` tagged with the render's id. On last frame, it does NOT `quit_message_loop()` (host stays alive for other renders) — instead it marks the render done and signals the host to finalize that render.

- [ ] **Step 1: Rewrite paint.rs**

Replace the entire contents of `src/renderer/paint.rs` with:

```rust
// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::host::ipc::emit_evt;
use crate::host::render::Host;
use crate::ipc::{self, Evt};
use crate::renderer::capture::schedule_invalidate;
use crate::renderer::host::decode_stego;
use cef::*;
use std::sync::Arc;

fn report_progress(id: u64, frame: u32, total: u32) {
    emit_evt(&Evt::Progress { id, frame, total });
}

wrap_render_handler! {
    pub struct CaptureHandler { pub host: Arc<Host> }
    impl RenderHandler {
        fn view_rect(&self, browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let (Some(browser), Some(rect)) = (browser, rect) else { return };
            let Some(render) = self.host.by_browser(browser.identifier()) else { return };
            let r = render.lock().expect("render poisoned");
            rect.x = 0;
            rect.y = 0;
            rect.width = r.width;
            rect.height = r.height + 1;
        }

        fn on_paint(
            &self,
            browser: Option<&mut Browser>,
            _paint_type: PaintElementType,
            _dirty: Option<&[Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            let Some(browser) = browser else { return };
            let Some(render) = self.host.by_browser(browser.identifier()) else { return };
            let mut r = render.lock().expect("render poisoned");
            if r.done || buffer.is_null() { return; }
            if !r.capture.budget_done { return; }
            let buf = unsafe {
                std::slice::from_raw_parts(buffer, (width as usize) * (height as usize) * 4)
            };

            const MAX_INVALIDATES: u32 = 60;
            let decoded = decode_stego(buf, width, height);
            let stego_match = decoded == Some(r.capture.requested_ms);
            if !stego_match {
                r.capture.stuck_invalidates += 1;
                emit_evt(&Evt::Console {
                    id: r.id, level: ipc::Level::Warn, source: "havi".into(),
                    message: format!(
                        "stuck frame={} req={} got={:?} retry={}/{}",
                        r.capture.next_frame, r.capture.requested_ms, decoded,
                        r.capture.stuck_invalidates, MAX_INVALIDATES
                    ),
                });
                if r.capture.stuck_invalidates > MAX_INVALIDATES {
                    let id = r.id;
                    let frame = r.capture.next_frame;
                    let ms = r.capture.requested_ms;
                    r.done = true;
                    r.tx = None;
                    drop(r);
                    emit_evt(&Evt::Error {
                        id,
                        message: format!("compositor stalled at frame {frame} ms={ms}"),
                    });
                    self.host.finish(id);
                    return;
                }
                let browser = r.browser.clone();
                drop(r);
                schedule_invalidate(browser);
                return;
            }

            let stride = (width as usize) * 4;
            let payload = r.height as usize * stride;
            let n = r.capture.next_frame;
            let total = r.total_frames;
            let last = n + 1 >= total;
            let frame_bytes = buf[..payload].to_vec();
            if let Some(tx) = r.tx.as_ref() {
                let _ = tx.send(frame_bytes);
            }
            let id = r.id;
            report_progress(id, n + 1, total);

            if last {
                r.done = true;
                r.tx = None;
                drop(r);
                self.host.finish(id);
            } else {
                r.capture.next_frame = n + 1;
                let render2 = render.clone();
                drop(r);
                crate::renderer::capture::schedule_step(&render2);
            }
        }
    }
}
```

`self.host.finish(id)` is defined in Task 9 (host lifecycle). For now it won't compile until Task 9; that's fine — we build the whole host module before wiring main.rs.

- [ ] **Step 2: Build check**

Run: `cargo build --bin havi 2>&1 | grep -E "error\[|error:" | head -20`
Expected: errors about `Host::finish` not found (defined in Task 9) and `load.rs`/`main.rs` old refs. `paint.rs`'s own syntax should be valid.

- [ ] **Step 3: Commit**

```bash
git add src/renderer/paint.rs
git commit -m "refactor(paint): route on_paint by browser id, emit tagged Evt"
```

---

## Task 7: Migrate `load.rs` to per-render routing

**Files:**
- Modify: `src/renderer/load.rs`

`DetLoadHandler` holds `Arc<Host>`. Phase state moves into `Render.phase`. `advance_phase` operates on a `RenderRef`. Tolerant timeout resolves the render by id (passed at construction).

- [ ] **Step 1: Rewrite load.rs**

Replace the entire contents of `src/renderer/load.rs` with:

```rust
// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use crate::host::ipc::emit_evt;
use crate::host::render::{Host, RenderRef};
use crate::ipc::{self, Evt, RenderId};
use crate::renderer::capture::{pause_virtual_time, schedule_step};
use cef::*;
use std::sync::Arc;

const WARMUP_MS: i64 = 2000;
pub const LOAD_TIMEOUT_MS: i64 = 15000;

wrap_task! {
    pub struct ReloadTask { pub browser: crate::renderer::capture::BrowserHandle }
    impl Task {
        fn execute(&self) {
            if let Some(main) = self.browser.lock().expect("browser poisoned")
                .as_ref().and_then(|b| b.main_frame())
            {
                let url = CefString::from(&main.url()).to_string();
                main.load_url(Some(&CefString::from(url.as_str())));
            }
        }
    }
}

wrap_task! {
    pub struct TolerantTimeoutTask { pub host: Arc<Host>, pub id: RenderId }
    impl Task {
        fn execute(&self) {
            let Some(render) = self.host.by_id(self.id) else { return };
            if render.lock().expect("render poisoned").phase >= 2 { return; }
            let tolerant = render.lock().expect("render poisoned").tolerant;
            if !tolerant {
                emit_evt(&Evt::Error {
                    id: self.id,
                    message: format!("load timeout after {LOAD_TIMEOUT_MS}ms (pass tolerant to proceed anyway)"),
                });
                self.host.finish(self.id);
                return;
            }
            emit_evt(&Evt::Console {
                id: self.id, level: ipc::Level::Warn, source: "havi".into(),
                message: format!("load timeout after {LOAD_TIMEOUT_MS}ms — proceeding tolerant"),
            });
            advance_phase(&self.host, &render, None);
            advance_phase(&self.host, &render, None);
        }
    }
}

wrap_task! {
    pub struct PrimeTask { pub render: RenderRef }
    impl Task {
        fn execute(&self) {
            let (browser, cdp) = {
                let r = self.render.lock().expect("render poisoned");
                (r.browser.clone(), r.cdp.clone())
            };
            if let Some(h) = browser.lock().expect("browser poisoned")
                .as_ref().and_then(|b| b.host())
            {
                pause_virtual_time(&cdp, &h);
            }
            schedule_step(&self.render);
        }
    }
}

/// Advance phase: 0→1 schedules warmup reload, 1→2 primes render.
pub fn advance_phase(host: &Arc<Host>, render: &RenderRef, iframe: Option<Frame>) {
    let phase = render.lock().expect("render poisoned").phase;
    match phase {
        0 => {
            render.lock().expect("render poisoned").phase = 1;
            ipc::set_console_capture(true);
            let browser = render.lock().expect("render poisoned").browser.clone();
            let mut task = ReloadTask::new(browser);
            post_delayed_task(ThreadId::UI, Some(&mut task), WARMUP_MS);
        }
        1 => {
            render.lock().expect("render poisoned").phase = 2;
            let f = iframe.or_else(|| {
                render.lock().expect("render poisoned").browser.lock().expect("browser")
                    .as_ref().and_then(|b| b.main_frame())
            });
            if let Some(ff) = f {
                *render.lock().expect("render poisoned").iframe.lock().expect("iframe") = Some(ff);
            }
            let _ = host;
            let mut task = PrimeTask::new(render.clone());
            post_task(ThreadId::UI, Some(&mut task));
        }
        _ => {}
    }
}

wrap_load_handler! {
    pub struct DetLoadHandler { pub host: Arc<Host> }

    impl LoadHandler {
        fn on_load_end(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _http_status_code: ::std::os::raw::c_int,
        ) {
            let Some(browser) = browser else { return };
            let Some(render) = self.host.by_browser(browser.identifier()) else { return };
            if render.lock().expect("render poisoned").tolerant { return; }
            let iframe = match frame { Some(f) if f.is_main() == 0 => f.clone(), _ => return };
            advance_phase(&self.host, &render, Some(iframe));
        }

        fn on_load_error(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let id = browser.and_then(|b| self.host.by_browser(b.identifier()))
                .map(|r| r.lock().expect("render poisoned").id)
                .unwrap_or(0);
            let msg = format!(
                "load failed code={:?} text={} url={}",
                error_code,
                error_text.map(|s| s.to_string()).unwrap_or_default(),
                failed_url.map(|s| s.to_string()).unwrap_or_default(),
            );
            emit_evt(&Evt::Error { id, message: msg });
        }
    }
}
```

DOM-ready (tolerant) hook resolution: `scheme::set_on_dom_ready` is process-global and has no browser context (Task 11 handles routing it to the right render; for now it's wired in host setup).

- [ ] **Step 2: Build check**

Run: `cargo build --bin havi 2>&1 | grep -E "error\[|error:" | head -20`
Expected: errors only about `Host::finish`/`Host::by_id` (Task 9 adds `finish`; `by_id` exists from Task 3) and main.rs old refs. load.rs syntax valid.

- [ ] **Step 3: Commit**

```bash
git add src/renderer/load.rs
git commit -m "refactor(load): per-render phase + browser-id routing"
```

---

## Task 8: Per-render encoder ownership

**Files:**
- Modify: `src/video/encoder.rs`

Add a helper that spawns ffmpeg and returns both the `SyncSender` and a join handle plus the pid, so a `Render` owns its own encoder. The existing `spawn` + `start_pipe` stay; add a combined `start(width,height,fps,out) -> EncoderHandle`.

- [ ] **Step 1: Add EncoderHandle + start**

Append to `src/video/encoder.rs`:

```rust
pub struct EncoderHandle {
    pub pid: u32,
    pub tx: SyncSender<Vec<u8>>,
    pub child: Child,
    pub pump: JoinHandle<()>,
}

pub fn start(width: i32, height: i32, fps: u32, out_path: &str) -> EncoderHandle {
    let mut child = spawn(width, height, fps, out_path);
    let pid = child.id();
    let (tx, pump) = start_pipe(&mut child);
    EncoderHandle { pid, tx, child, pump }
}

impl EncoderHandle {
    pub fn finish(mut self) -> std::io::Result<std::process::ExitStatus> {
        drop(self.tx);
        let _ = self.pump.join();
        let status = self.child.wait();
        crate::sandbox::unregister_ffmpeg(self.pid);
        status
    }
}
```

`SyncSender`, `Child`, `JoinHandle` are already imported at the top of `encoder.rs`.

- [ ] **Step 2: Build check**

Run: `cargo build --lib 2>&1 | grep -E "error\[|error:" | head -10`
Expected: no new errors from encoder.rs (lib builds the encoder module; host wiring errors are separate, but `--lib` without napi feature still compiles renderer? No — `--lib` builds everything. Use `--bin havi` once host exists). For now:

Run: `cargo build --lib 2>&1 | grep "encoder" | head`
Expected: empty (no encoder errors).

- [ ] **Step 3: Commit**

```bash
git add src/video/encoder.rs
git commit -m "feat(encoder): EncoderHandle for per-render ffmpeg ownership"
```

---

## Task 9: Host lifecycle (start / finish / cancel / queue)

**Files:**
- Modify: `src/host/render.rs` (add lifecycle methods)
- Create: `src/host/run.rs`
- Modify: `src/host/mod.rs`

This is the core. `Host` gains:
- `max_parallel` + a pending queue.
- `start(id, opts)` — if under cap, create browser + encoder + register; else enqueue.
- `finish(id)` — finalize encoder (flush mp4), emit `Done`, remove render, drain queue.
- `cancel(id)` — kill encoder, close browser, emit `Error{cancelled}`, remove, drain.

Browser creation needs the shared `RequestContext` + client + host page. We pass a `BrowserFactory` closure into `Host` so `render.rs` stays CEF-free where possible — but realistically browser creation is CEF-heavy, so it lives in `run.rs`. `Host` calls back into a stored factory.

- [ ] **Step 1: Extend Host in render.rs**

Replace the `Host` struct + impl in `src/host/render.rs` with:

```rust
pub type StartFn = Box<dyn Fn(&Arc<Host>, RenderId, crate::api::RenderOpts) + Send + Sync>;

pub struct Host {
    renders: Mutex<HashMap<RenderId, RenderRef>>,
    by_browser: Mutex<HashMap<i32, RenderId>>,
    queue: Mutex<std::collections::VecDeque<(RenderId, crate::api::RenderOpts)>>,
    max_parallel: usize,
    start_fn: Mutex<Option<StartFn>>,
}

impl Host {
    pub fn new(max_parallel: usize) -> Arc<Self> {
        Arc::new(Self {
            renders: Mutex::new(HashMap::new()),
            by_browser: Mutex::new(HashMap::new()),
            queue: Mutex::new(std::collections::VecDeque::new()),
            max_parallel: max_parallel.max(1),
            start_fn: Mutex::new(None),
        })
    }

    pub fn set_start_fn(&self, f: StartFn) {
        *self.start_fn.lock().expect("start_fn poisoned") = Some(f);
    }

    pub fn insert(&self, render: RenderRef) {
        let id = render.lock().expect("render poisoned").id;
        self.renders.lock().expect("renders poisoned").insert(id, render);
    }

    pub fn bind_browser(&self, browser_id: i32, render_id: RenderId) {
        self.by_browser.lock().expect("by_browser poisoned").insert(browser_id, render_id);
    }

    pub fn by_id(&self, id: RenderId) -> Option<RenderRef> {
        self.renders.lock().expect("renders poisoned").get(&id).cloned()
    }

    pub fn by_browser(&self, browser_id: i32) -> Option<RenderRef> {
        let id = *self.by_browser.lock().expect("by_browser poisoned").get(&browser_id)?;
        self.by_id(id)
    }

    pub fn active_count(&self) -> usize {
        self.renders.lock().expect("renders poisoned").len()
    }

    pub fn submit(self: &Arc<Self>, id: RenderId, opts: crate::api::RenderOpts) {
        let at_cap = self.active_count() >= self.max_parallel;
        if at_cap {
            self.queue.lock().expect("queue poisoned").push_back((id, opts));
            return;
        }
        self.spawn_render(id, opts);
    }

    fn spawn_render(self: &Arc<Self>, id: RenderId, opts: crate::api::RenderOpts) {
        let f = self.start_fn.lock().expect("start_fn poisoned");
        if let Some(start) = f.as_ref() {
            start(self, id, opts);
        }
    }

    fn drain_queue(self: &Arc<Self>) {
        let next = self.queue.lock().expect("queue poisoned").pop_front();
        if let Some((id, opts)) = next {
            self.spawn_render(id, opts);
        }
    }

    pub fn finish(self: &Arc<Self>, id: RenderId) {
        let Some(render) = self.remove(id) else { return };
        let (encoder, out, frames, started, width, height, fps, errored) = {
            let mut r = render.lock().expect("render poisoned");
            r.done = true;
            r.tx = None;
            (
                r.encoder.take(),
                r.out.clone(),
                r.capture.next_frame,
                r.started_at,
                r.width, r.height,
                (1000.0 / r.frame_ms).round() as u32,
                r.errored,
            )
        };
        if let Some(b) = render.lock().expect("render poisoned").browser.lock().expect("browser").take() {
            if let Some(h) = b.host() { h.close_browser(true); }
        }
        let mut ok = true;
        if let Some(enc) = encoder {
            ok = enc.finish().map(|s| s.success()).unwrap_or(false);
        }
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if !errored {
            if ok {
                emit_evt(&Evt::Done { id, out, frames, elapsed_ms });
            } else {
                emit_evt(&Evt::Error { id, message: "ffmpeg encoder failed".into() });
            }
        }
        let _ = (width, height, fps);
        self.drain_queue();
    }

    pub fn cancel(self: &Arc<Self>, id: RenderId) {
        let Some(render) = self.by_id(id) else { return };
        {
            let mut r = render.lock().expect("render poisoned");
            r.errored = true;
            r.done = true;
            r.tx = None;
        }
        emit_evt(&Evt::Error { id, message: "cancelled".into() });
        self.remove(id);
        if let Some(enc) = render.lock().expect("render poisoned").encoder.take() {
            let _ = enc.finish();
        }
        if let Some(b) = render.lock().expect("render poisoned").browser.lock().expect("browser").take() {
            if let Some(h) = b.host() { h.close_browser(true); }
        }
        self.drain_queue();
    }

    fn remove(&self, id: RenderId) -> Option<RenderRef> {
        self.by_browser.lock().expect("by_browser poisoned").retain(|_, v| *v != id);
        self.renders.lock().expect("renders poisoned").remove(&id)
    }
}
```

Update `Render` struct fields: replace `pub encoder_pid: Option<u32>` and `pub tx: ...` with an `encoder: Option<EncoderHandle>` plus keep `tx` derived from it, and add `errored`. Simplest: store both `tx` (clone of encoder's sender) and `encoder`. Rewrite `Render`:

```rust
use crate::video::encoder::EncoderHandle;

pub struct Render {
    pub id: RenderId,
    pub width: i32,
    pub height: i32,
    pub total_frames: u32,
    pub frame_ms: f64,
    pub out: String,
    pub tolerant: bool,
    pub phase: u8,
    pub browser: BrowserHandle,
    pub iframe: FrameHandle,
    pub cdp: Cdp,
    pub capture: CaptureState,
    pub encoder: Option<EncoderHandle>,
    pub tx: Option<SyncSender<Vec<u8>>>,
    pub done: bool,
    pub errored: bool,
    pub started_at: Instant,
}
```

Add `use crate::host::ipc::emit_evt;` and `use crate::ipc::Evt;` to the top of `render.rs`.

Fix the test `fake_render` to match new fields (`encoder: None`, `errored: false`).

- [ ] **Step 2: Build check (render.rs)**

Run: `cargo build --lib 2>&1 | grep -E "host/render|host::render" | head -20`
Expected: only errors from `EncoderHandle` import path if encoder isn't `pub` — confirm `pub struct EncoderHandle` (Task 8). Resolve any field mismatches.

- [ ] **Step 3: Create run.rs with the StartFn factory**

Create `src/host/run.rs`:

```rust
// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

use crate::api::RenderOpts;
use crate::cef::cdp::{Cdp, CdpObserver};
use crate::cef::client::DetClient;
use crate::host::ipc::{emit_evt, parse_cmd};
use crate::host::render::{CaptureState, Host, Render};
use crate::ipc::{Cmd, Evt, RenderId};
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
        started_at: Instant::now(),
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

    let mut window_info = WindowInfo::default().set_as_windowless(Default::default());
    let _ = &mut window_info;
    let mut bs = BrowserSettings::default();
    bs.windowless_frame_rate = 240;
    bs.background_color = 0;

    let b = browser_host_create_browser_sync(
        Some(&window_info),
        Some(&mut client),
        Some(&CefString::from(host_url.as_str())),
        Some(&bs),
        None, None,
    ).expect("create browser");

    let browser_id = b.identifier();
    if let Some(bh) = b.host() {
        bh.was_resized();
        let mut obs = CdpObserver::new(cdp.clone());
        let _ = bh.add_dev_tools_message_observer(Some(&mut obs));
        std::mem::forget(obs);
    }
    *browser.lock().expect("browser poisoned") = Some(b);
    host.bind_browser(browser_id, id);

    emit_evt(&Evt::Started { id });

    let mut tt = TolerantTimeoutTask::new(host.clone(), id);
    post_delayed_task(ThreadId::UI, Some(&mut tt), LOAD_TIMEOUT_MS);
}
```

Note: `std::mem::forget(obs)` keeps the observer alive for the browser's lifetime. The original code held `_devtools` registration in `main.rs`; in the multi-render model the registration handle is per-browser and must outlive the function. Storing it in `Render` would be cleaner — if `add_dev_tools_message_observer` returns a `Registration`, store it in `Render.devtools_reg`. Adjust: add `pub devtools: Option<Registration>` to `Render` and assign instead of forget. Confirm the return type via `cargo doc`/signature; if it returns `Option<Registration>`, store it.

- [ ] **Step 4: Register run module**

In `src/host/mod.rs`:

```rust
// Created by Autokaka (qq1909698494@gmail.com) on 2026/06/11.

pub mod ipc;
pub mod render;
pub mod run;
```

- [ ] **Step 5: Build check**

Run: `cargo build --bin havi 2>&1 | grep -E "error\[|error:" | head -30`
Expected: errors only in `main.rs` (still old). host module compiles. Resolve any signature mismatches in `start_render` (browser create args, observer registration type) until host module is clean.

- [ ] **Step 6: Commit**

```bash
git add src/host/ src/video/encoder.rs
git commit -m "feat(host): lifecycle (submit/finish/cancel/queue) + browser factory"
```

---

## Task 10: Host event loop + command dispatch in run.rs

**Files:**
- Modify: `src/host/run.rs`

Add the `run()` entrypoint: spawn a stdin reader thread that parses `Cmd` lines and posts them to the CEF UI thread (CEF objects must be touched on UI thread), emit `HostReady`, run the message loop, handle `Shutdown`/EOF.

- [ ] **Step 1: Add run() and command posting**

Append to `src/host/run.rs`:

```rust
wrap_task! {
    pub struct CmdTask { pub host: Arc<Host>, pub cmd: Mutex<Option<Cmd>> }
    impl Task {
        fn execute(&self) {
            let Some(cmd) = self.cmd.lock().expect("cmd poisoned").take() else { return };
            match cmd {
                Cmd::Start { id, opts } => self.host.submit(id, opts),
                Cmd::Cancel { id } => self.host.cancel(id),
                Cmd::Shutdown => {
                    emit_evt(&Evt::HostExit);
                    quit_message_loop();
                }
            }
        }
    }
}

fn post_cmd(host: &Arc<Host>, cmd: Cmd) {
    let mut task = CmdTask::new(host.clone(), Mutex::new(Some(cmd)));
    post_task(ThreadId::UI, Some(&mut task));
}

pub fn run(host: Arc<Host>) {
    install_start_fn(&host);

    {
        let host = host.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                if let Some(cmd) = parse_cmd(&line) {
                    post_cmd(&host, cmd);
                }
            }
            // stdin EOF = implicit shutdown
            post_cmd(&host, Cmd::Shutdown);
        });
    }

    emit_evt(&Evt::HostReady);
    run_message_loop();
}
```

`CmdTask` uses `Mutex<Option<Cmd>>` because `wrap_task!` requires the task struct to be `Send + Sync` and `Cmd` is moved out once on execute.

- [ ] **Step 2: Build check**

Run: `cargo build --bin havi 2>&1 | grep -E "error\[|error:" | head -20`
Expected: only main.rs errors remain.

- [ ] **Step 3: Commit**

```bash
git add src/host/run.rs
git commit -m "feat(host): stdin command loop + UI-thread dispatch + HostReady"
```

---

## Task 11: Rewire main.rs onto the host

**Files:**
- Modify: `src/main.rs`
- Modify: `src/cli.rs`

`main.rs` shrinks to: CEF subprocess bootstrap (unchanged), CEF init with shared `RequestContext`/`root_cache_path`, scheme registration, then either:
- `--host` flag set → `host::run::run(host)` (daemon mode).
- otherwise (CLI one-shot) → submit one render built from CLI args, run loop, exit on its `Done`.

The CLI one-shot still needs a "quit when this render finishes" behavior. We give `Host` an optional `single_shot: Option<RenderId>` that calls `quit_message_loop()` from `finish`/`cancel` when the finishing id matches and the queue is empty.

- [ ] **Step 1: Add --host flag + make CLI fields optional-friendly**

In `src/cli.rs`, add the flag and make `source` optional (host mode has no source):

```rust
#[derive(Parser, Debug)]
#[command(name = "havi", about = "Deterministic HTML-to-video renderer.")]
pub struct Cli {
    /// file://, http(s)://, data: URI, or filesystem path (relative or absolute)
    pub source: Option<String>,
    #[arg(short = 'W', long, default_value_t = 1920)]
    pub width: i32,
    #[arg(short = 'H', long, default_value_t = 1080)]
    pub height: i32,
    #[arg(short, long, default_value_t = 30)]
    pub fps: u32,
    /// Duration in seconds.
    #[arg(short = 't', long, default_value_t = 5)]
    pub duration: u32,
    #[arg(short, long, default_value = "out.mp4")]
    pub out: String,
    /// On load timeout, proceed with partial DOM instead of erroring out.
    #[arg(long)]
    pub tolerant: bool,
    /// HTTP proxy rules (JSON array).
    #[arg(long)]
    pub proxy: Option<String>,
    /// Daemon mode: read JSON commands on stdin, emit events on stdout.
    #[arg(long)]
    pub host: bool,
}
```

- [ ] **Step 2: Add single_shot to Host**

In `src/host/render.rs`, add a field and a setter, and call quit in `finish`/`cancel` when matched:

Add to `Host` struct: `single_shot: Mutex<Option<RenderId>>,` initialized `Mutex::new(None)` in `new`.

Add method:

```rust
pub fn set_single_shot(&self, id: RenderId) {
    *self.single_shot.lock().expect("single_shot poisoned") = Some(id);
}

fn maybe_quit_single_shot(&self, finished: RenderId) {
    let target = *self.single_shot.lock().expect("single_shot poisoned");
    if target == Some(finished)
        && self.active_count() == 0
        && self.queue.lock().expect("queue poisoned").is_empty()
    {
        cef::quit_message_loop();
    }
}
```

Call `self.maybe_quit_single_shot(id);` at the end of both `finish` and `cancel` (after `drain_queue`). Add `use cef::quit_message_loop;` if needed (or fully-qualify as above).

- [ ] **Step 3: Rewrite main.rs**

Replace the contents of `src/main.rs` with:

```rust
// Created by Autokaka (qq1909698494@gmail.com) on 2026/05/26.

use cef::*;
use clap::Parser;
use havi_core::api::RenderOpts;
use havi_core::cef::app::{init_api, make_app};
use havi_core::cli::Cli;
use havi_core::host::render::Host;
use havi_core::host::run::{max_parallel, run};
use havi_core::video::encoder;
use havi_core::video::scheme::{self, HaviFrameFactory, SCHEME};
use std::sync::Arc;

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

    // Tolerant DOM-ready hook (single active render assumption for tolerant mode).
    {
        let host = host.clone();
        scheme::set_on_dom_ready(move || {
            host.dom_ready_advance();
        });
    }

    if !cli.host {
        let source = cli.source.clone().unwrap_or_else(|| {
            eprintln!("error: source required (or pass --host)");
            std::process::exit(2);
        });
        let opts = RenderOpts {
            source,
            out: Some(cli.out.clone()),
            width: Some(cli.width),
            height: Some(cli.height),
            fps: Some(cli.fps),
            duration: Some(cli.duration),
            tolerant: Some(cli.tolerant),
            proxy: cli.proxy.as_ref().map(|s| {
                serde_json::from_str(s).expect("invalid --proxy JSON")
            }),
        };
        havi_core::host::run::install_start_fn(&host);
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
```

`host.dom_ready_advance()` resolves the most-recent in-flight tolerant render. Add to `Host` in `render.rs`:

```rust
pub fn dom_ready_advance(self: &Arc<Self>) {
    // Tolerant mode runs one render at a time in practice; advance the
    // single active tolerant render if present.
    let target = {
        let renders = self.renders.lock().expect("renders poisoned");
        renders.values()
            .find(|r| { let g = r.lock().expect("render poisoned"); g.tolerant && g.phase < 2 })
            .cloned()
    };
    if let Some(render) = target {
        crate::renderer::load::advance_phase(self, &render, None);
    }
}
```

Note: `cef_init` now sets BOTH `root_cache_path` and `cache_path` to the same profile dir — the "share global browser cache" pattern. Since only the host process touches this path, the SingletonLock wipe stays as belt-and-suspenders for crash recovery.

- [ ] **Step 4: Full build**

Run: `cargo build --bin havi 2>&1 | tail -30`
Expected: clean build (warnings OK). Fix any remaining signature/borrow errors.

- [ ] **Step 5: Smoke test — CLI one-shot**

Run: `cargo build --bin havi 2>&1 | tail -3 && ./target/debug/havi stamp.html -t 2 -o /tmp/smoke.mp4 -W 320 -H 240 2>&1 | tail -20`
Expected: renders, exits, `/tmp/smoke.mp4` exists. Verify:

Run: `ls -la /tmp/smoke.mp4 && ./target/debug/../release/ffmpeg -i /tmp/smoke.mp4 2>&1 | grep Duration || true`
Expected: file > 0 bytes. (ffmpeg path may differ in debug; just check file exists and is non-empty.)

- [ ] **Step 6: Commit**

```bash
git add src/main.rs src/cli.rs src/host/render.rs
git commit -m "feat(host): main.rs drives host; CLI one-shot + --host daemon; shared cache"
```

---

## Task 12: Two-render parallel smoke test (host daemon)

**Files:**
- Create: `scripts/smoke_two.sh` (throwaway test helper, not shipped)

Verify the daemon runs two concurrent renders and emits correctly-tagged events.

- [ ] **Step 1: Create the smoke script**

Create `scripts/smoke_two.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
BIN="${1:-./target/debug/havi}"
SRC="${2:-stamp.html}"
SRC_ABS="$(cd "$(dirname "$SRC")" && pwd)/$(basename "$SRC")"

printf '%s\n' \
  "{\"cmd\":\"start\",\"id\":1,\"opts\":{\"source\":\"$SRC_ABS\",\"out\":\"/tmp/a.mp4\",\"width\":320,\"height\":240,\"duration\":2}}" \
  "{\"cmd\":\"start\",\"id\":2,\"opts\":{\"source\":\"$SRC_ABS\",\"out\":\"/tmp/b.mp4\",\"width\":320,\"height\":240,\"duration\":2}}" \
  | { cat; sleep 30; } \
  | "$BIN" --host
```

The `sleep 30` keeps stdin open while renders run; EOF after triggers shutdown. Crude but works for smoke. (Real parent keeps the pipe open until it sends `shutdown`.)

- [ ] **Step 2: Run the smoke test**

Run: `chmod +x scripts/smoke_two.sh && ./scripts/smoke_two.sh 2>&1 | tail -40`
Expected: stdout shows `{"evt":"host_ready"}`, `{"evt":"started","id":1}`, `{"evt":"started","id":2}`, interleaved `progress` for both ids, two `done` events, then `host_exit`.

Verify outputs:

Run: `ls -la /tmp/a.mp4 /tmp/b.mp4`
Expected: both non-empty.

If renders serialize (id 2 starts only after id 1 done), check `max_parallel()` returns ≥2 and `submit` isn't accidentally enqueuing. Debug before proceeding.

- [ ] **Step 3: Commit**

```bash
git add scripts/smoke_two.sh
git commit -m "test(host): two-render parallel smoke script"
```

---

## Task 13: napi binding rewires to persistent host

**Files:**
- Modify: `src/napi/mod.rs`
- Modify: `src/api.rs` (host-client helpers)

Replace per-call `api::spawn` (one subprocess per render) with a lazily-spawned shared host. `HostClient` owns the host child, a stdin writer behind a `Mutex`, an atomic id counter, and a `Mutex<HashMap<RenderId, Sender<Evt>>>` registry. A reader thread parses host stdout and dispatches each `Evt` to the matching render's channel.

- [ ] **Step 1: Add HostClient to api.rs**

Replace the `RenderHandle`/`spawn`/`locate_bin` section of `src/api.rs` (keep `RenderOpts` + `self_dylib_dir`). New content for the client:

```rust
use crate::ipc::{Cmd, Evt, RenderId};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

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
                // host died — fail all pending
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
```

Keep `locate_bin()` and both `self_dylib_dir()` impls from the existing file unchanged. Remove the old `RenderHandle`, `spawn`, `pid`, `try_recv`, `recv`, `cancel`, `wait`.

- [ ] **Step 2: Rewrite napi/mod.rs render()**

Replace `src/napi/mod.rs` body (keep `render_help`, the `#[napi(object)]` structs, `ProgressTsfn`/`ConsoleTsfn`, `AbortState`, `RenderInput` + `FromNapiValue`). Replace the `render` fn + `cancel_pid`:

```rust
use crate::api::HostClient;
use crate::ipc::{Evt, RenderId};
use std::sync::OnceLock;

static HOST: OnceLock<Arc<HostClient>> = OnceLock::new();

fn host() -> Result<Arc<HostClient>> {
    if let Some(h) = HOST.get() { return Ok(h.clone()); }
    let h = HostClient::spawn().map_err(|e| Error::from_reason(e.to_string()))?;
    let _ = HOST.set(h.clone());
    Ok(HOST.get().cloned().unwrap_or(h))
}

#[derive(Default)]
struct AbortState {
    flag: AtomicBool,
    id: AtomicU64,
}

#[napi]
pub fn render<'env>(
    env: &'env Env,
    input: RenderInput,
) -> Result<PromiseRaw<'env, RenderResult>> {
    let client = host()?;
    let (id, rx) = client.begin(input.options).map_err(|e| Error::from_reason(e.to_string()))?;
    input.abort_state.id.store(id, Ordering::SeqCst);
    if input.abort_state.flag.load(Ordering::SeqCst) {
        client.cancel(id);
    }
    let abort_state = input.abort_state.clone();
    let on_progress = input.on_progress;
    let on_console = input.on_console;
    let client2 = client.clone();

    env.spawn_future(async move {
        let result = loop {
            let evt = napi::tokio::task::block_in_place(|| rx.recv().ok());
            match evt {
                Some(Evt::Progress { frame, total, .. }) => {
                    if let Some(cb) = &on_progress {
                        cb.call(ProgressEvent { frame, total }, ThreadsafeFunctionCallMode::NonBlocking);
                    }
                }
                Some(Evt::Console { level, source, message, .. }) => {
                    if let Some(cb) = &on_console {
                        let level = match level {
                            ipc::Level::Info => "info",
                            ipc::Level::Warn => "warn",
                            ipc::Level::Error => "error",
                        }.to_string();
                        cb.call(ConsoleEvent { level, source, message }, ThreadsafeFunctionCallMode::NonBlocking);
                    }
                }
                Some(Evt::Started { .. }) => {}
                Some(Evt::Done { frames, out, elapsed_ms, .. }) => {
                    break Ok(RenderResult {
                        frames,
                        width: 0, height: 0, fps: 0,
                        out,
                        elapsed_ms: u32::try_from(elapsed_ms).unwrap_or(u32::MAX),
                    });
                }
                Some(Evt::Error { message, .. }) => break Err(Error::from_reason(message)),
                Some(Evt::HostReady) | Some(Evt::HostExit) => {}
                None => break Err(Error::from_reason("render channel closed")),
            }
        };
        client2.forget(id);
        result
    })
}
```

Note: `RenderResult.width/height/fps` are no longer in the `Done` event (the napi caller already knows them from `opts`). Either (a) keep them 0 and document, or (b) add them back to `Evt::Done`. **Decision: keep `Evt::Done` minimal; fill width/height/fps from the input opts.** To do that, capture them before `spawn_future`:

Insert before `env.spawn_future`:
```rust
let (rw, rh, rfps) = (
    input_width, input_height, input_fps   // captured from opts
);
```

But `input.options` is moved into `begin`. So capture them first:

```rust
let client = host()?;
let opts = input.options;
let (rw, rh, rfps) = (opts.width_or(), opts.height_or(), opts.fps_or());
let (id, rx) = client.begin(opts).map_err(|e| Error::from_reason(e.to_string()))?;
```

Then in `Done`: `width: rw, height: rh, fps: rfps,`. Move the `let (rw,rh,rfps)` capture into the closure via `move`.

Update `RenderInput::from_napi_value` `abort_state` `on_abort` closure: it currently calls `cancel_pid(p)` using a pid. Change to store id and call `HOST`'s cancel. Since `on_abort` fires on a JS thread without the client handle, route via the global `HOST`:

```rust
s.on_abort(move || {
    state.flag.store(true, Ordering::SeqCst);
    let id = state.id.load(Ordering::SeqCst);
    if id > 0 {
        if let Some(h) = HOST.get() { h.cancel(id); }
    }
});
```

Delete `cancel_pid` entirely.

- [ ] **Step 3: Build the napi lib**

Run: `cargo build --lib --features napi-binding 2>&1 | tail -30`
Expected: clean build. Fix borrow/move errors around `opts` capture.

- [ ] **Step 4: Commit**

```bash
git add src/napi/mod.rs src/api.rs
git commit -m "feat(napi): persistent shared host client + per-call event fan-out"
```

---

## Task 14: Build the .node + parallel JS smoke test

**Files:**
- Create: `scripts/smoke_parallel.mjs` (throwaway)

Build the native addon and verify `Promise.all` of two renders runs concurrently through one host.

- [ ] **Step 1: Build the addon for the host platform**

Run: `./build.ts darwin-arm64 2>&1 | tail -30` (or the project's build entry; if `build.ts` builds all targets, scope to host). If the build script needs a specific invocation, check:

Run: `head -60 build.ts | grep -nE "process.argv|target|HOST_TARGET"`
Expected: shows how to pass a single target. Use that. The goal: produce `dist/darwin-arm64/` with the `havi` binary, `.node`, `ffmpeg`, and CEF bundle.

- [ ] **Step 2: Create parallel smoke**

Create `scripts/smoke_parallel.mjs`:

```javascript
import { havi } from "../havi/index.js";

const mk = (out) => havi.render({
  options: { source: new URL("../stamp.html", import.meta.url).pathname,
             out, width: 320, height: 240, duration: 2 },
  onProgress: (e) => process.stdout.write(`\r${out}: ${e.frame}/${e.total}   `),
});

const t0 = Date.now();
const [a, b] = await Promise.all([mk("/tmp/p1.mp4"), mk("/tmp/p2.mp4")]);
console.log(`\nboth done in ${Date.now() - t0}ms`, a.out, b.out);
```

- [ ] **Step 3: Run it**

Run: `node scripts/smoke_parallel.mjs 2>&1 | tail -20`
Expected: both renders finish; wall-clock noticeably less than 2× a single render. Both `/tmp/p1.mp4` and `/tmp/p2.mp4` exist and are non-empty.

Run: `ls -la /tmp/p1.mp4 /tmp/p2.mp4`
Expected: both non-empty.

- [ ] **Step 4: Commit**

```bash
git add scripts/smoke_parallel.mjs
git commit -m "test(napi): parallel render smoke via shared host"
```

---

## Task 15: Graceful host shutdown on Node exit

**Files:**
- Modify: `src/api.rs` (HostClient Drop)

When the Node process exits, the `HostClient` drops. Send `shutdown` so the host flushes in-flight renders and exits cleanly instead of relying on stdin-EOF + parent-death watcher.

- [ ] **Step 1: Add Drop**

Add to `src/api.rs`:

```rust
impl Drop for HostClient {
    fn drop(&mut self) {
        if let Ok(mut w) = self.stdin.lock() {
            let _ = writeln!(w, r#"{{"cmd":"shutdown"}}"#);
            let _ = w.flush();
        }
        // child stdin closes on drop → host sees EOF as backstop
    }
}
```

Since `HostClient` lives in a `OnceLock` for the process lifetime, Drop fires at process teardown. The host's stdin-EOF watcher + parent-death watcher are the real safety net; this is the clean path.

- [ ] **Step 2: Build check**

Run: `cargo build --lib --features napi-binding 2>&1 | tail -10`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/api.rs
git commit -m "feat(napi): graceful host shutdown on client drop"
```

---

## Task 16: Cancellation correctness test

**Files:**
- Create: `scripts/smoke_cancel.mjs` (throwaway)

Verify AbortSignal cancels one render without affecting a sibling.

- [ ] **Step 1: Create the test**

Create `scripts/smoke_cancel.mjs`:

```javascript
import { havi } from "../havi/index.js";

const src = new URL("../stamp.html", import.meta.url).pathname;
const ctrl = new AbortController();

const cancelled = havi.render({
  options: { source: src, out: "/tmp/c1.mp4", width: 320, height: 240, duration: 10 },
  signal: ctrl.signal,
}).then(() => "resolved").catch((e) => `rejected: ${e.message}`);

const survivor = havi.render({
  options: { source: src, out: "/tmp/c2.mp4", width: 320, height: 240, duration: 2 },
});

setTimeout(() => ctrl.abort(), 500);

const [c, s] = await Promise.all([cancelled, survivor]);
console.log("cancelled render:", c);
console.log("survivor frames:", s.frames, s.out);
```

- [ ] **Step 2: Run it**

Run: `node scripts/smoke_cancel.mjs 2>&1 | tail -10`
Expected: `cancelled render: rejected: cancelled` (or "host..."/abort message); `survivor frames: 60 /tmp/c2.mp4`. Survivor completes normally despite sibling cancel.

Run: `ls -la /tmp/c2.mp4`
Expected: non-empty (survivor output intact).

- [ ] **Step 3: Commit**

```bash
git add scripts/smoke_cancel.mjs
git commit -m "test(napi): cancel one render leaves sibling intact"
```

---

## Task 17: Cache-reuse verification

**Files:** none (verification only)

Confirm Phase-4 goal: second render of the same URL in one host hits warm cache.

- [ ] **Step 1: Time two sequential renders of a network URL**

Use a URL with real fetches (the stamp.html if it loads remote assets, else a data-heavy page). Run twice through the daemon in one session:

Run:
```bash
SRC="$(pwd)/stamp.html"
printf '%s\n' \
  "{\"cmd\":\"start\",\"id\":1,\"opts\":{\"source\":\"$SRC\",\"out\":\"/tmp/w1.mp4\",\"width\":320,\"height\":240,\"duration\":2}}" \
  | { cat; sleep 8; printf '%s\n' "{\"cmd\":\"start\",\"id\":2,\"opts\":{\"source\":\"$SRC\",\"out\":\"/tmp/w2.mp4\",\"width\":320,\"height\":240,\"duration\":2}}"; sleep 8; } \
  | ./target/debug/havi --host 2>&1 | grep -E "started|done"
```
Expected: both renders complete. Compare elapsed_ms in the two `done` events — second should be ≤ first (cache warm). For a purely-local stamp.html the delta may be small; the real signal is "no SingletonLock error and both share one profile dir."

- [ ] **Step 2: Confirm single profile dir, single lock holder**

Run: `ls -la $(./target/debug/havi --host < /dev/null 2>/dev/null; find . -name SingletonLock 2>/dev/null) 2>/dev/null; echo "check sandbox/profile exists"`

Simpler check:

Run: `find . -path '*/sandbox/profile' -type d 2>/dev/null`
Expected: one `sandbox/profile` dir under the binary's dir. Only the host process ever locks it.

- [ ] **Step 3: No commit (verification only)**

---

## Task 18: Delete dead code + final cleanup

**Files:**
- Modify: `src/common/ipc.rs`, `src/renderer/paint.rs`, others as flagged

Now that the host emits `Evt` directly, the old human-mode progress bar path and `Msg`-based `ipc::emit/console/error` may be partly dead. The CLI one-shot still uses the host (which emits `Evt` to stdout when `HAVI_IPC=1`). But CLI mode is NOT `HAVI_IPC=1` — it's a human terminal. **Decision:** CLI one-shot should show the indicatif progress bar. Wire `emit_evt` to fall back to human rendering when `ipc::enabled()` is false.

- [ ] **Step 1: Make emit_evt human-aware**

In `src/host/ipc.rs`, change `emit_evt`:

```rust
use crate::ipc::{self, Cmd, Evt};
use std::io::Write;

pub fn emit_evt(evt: &Evt) {
    if ipc::enabled() {
        let Ok(line) = serde_json::to_string(evt) else { return };
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
        return;
    }
    human_render(evt);
}

fn human_render(evt: &Evt) {
    match evt {
        Evt::Progress { frame, total, .. } => ipc::human_progress(*frame, *total),
        Evt::Console { level, message, .. } => {
            let lvl = match level { ipc::Level::Info => "info", ipc::Level::Warn => "warn", ipc::Level::Error => "error" };
            ipc::human_log(&format!("{lvl}: {message}"));
        }
        Evt::Error { message, .. } => ipc::human_log(&format!("error: {message}")),
        Evt::Done { out, frames, elapsed_ms, .. } => {
            ipc::human_log(&format!("done: {out} ({frames} frames) in {:.2}s", *elapsed_ms as f64 / 1000.0));
        }
        Evt::Started { .. } | Evt::HostReady | Evt::HostExit => {}
    }
}
```

Add to `src/common/ipc.rs` two public helpers reusing the existing progress bar + log machinery:

```rust
pub fn human_progress(done: u32, total: u32) {
    let bar = bar_get_or_init(total);
    bar.set_position(done as u64);
    if done >= total { bar.finish(); }
}

fn bar_get_or_init(total: u32) -> ProgressBar {
    static PB: OnceLock<ProgressBar> = OnceLock::new();
    PB.get_or_init(|| {
        let pb = ProgressBar::new(total as u64);
        pb.set_style(
            indicatif::ProgressStyle::with_template("[{bar:40}] {pos}/{len} ({percent}%)")
                .unwrap().progress_chars("#-"),
        );
        set_progress_bar(pb.clone());
        pb
    }).clone()
}

pub fn human_log(line: &str) { log_line(line); }
```

(`log_line` already exists in ipc.rs; make it usable. `set_progress_bar`, `BAR` already there.)

The old `draw_progress`/`PROGRESS` static in `paint.rs` is now unused — already removed in Task 6 rewrite. Confirm no references remain.

- [ ] **Step 2: Remove now-dead Msg-based functions if unreferenced**

Run: `grep -rn "ipc::emit\|ipc::Msg\|Msg::Progress\|Msg::Done" src/ | grep -v "common/ipc.rs"`
Expected: empty (no external users). If empty, delete the `Msg` enum and `emit`, `error`, `console` fns from `ipc.rs` — replaced by `Evt`/`emit_evt`. If `console`/`error` still referenced (e.g., in `encoder.rs`, `session.rs`, `client.rs`, `load.rs`), keep them but have them route through `Evt` where a render id is available, or keep `Console` for id-less host-level logs with id=0.

Check remaining `ipc::console`/`ipc::error` callers:

Run: `grep -rn "ipc::console\|ipc::error" src/`
Expected list: `encoder.rs` (ffmpeg stderr), `session.rs` (ffprobe fail), `client.rs` (console forward). These are id-less. **Decision:** keep `ipc::console`/`ipc::error` as id=0 host-level logs — change their bodies to call `emit_evt(&Evt::Console{id:0,..})` / `emit_evt(&Evt::Error{id:0,..})`. This unifies on one wire format.

Rewrite `ipc::console` and `ipc::error` in `ipc.rs`:

```rust
pub fn error(message: &str) {
    crate::host::ipc::emit_evt(&Evt::Error { id: 0, message: message.to_string() });
}

pub fn console(level: Level, source: &str, message: &str) {
    crate::host::ipc::emit_evt(&Evt::Console {
        id: 0, level, source: source.to_string(), message: message.to_string(),
    });
}
```

Remove the now-unused `emit`/`Msg` if nothing references them.

- [ ] **Step 3: Full build both targets**

Run: `cargo build --bin havi 2>&1 | tail -5 && cargo build --lib --features napi-binding 2>&1 | tail -5`
Expected: both clean.

- [ ] **Step 4: Re-run smoke tests**

Run: `./target/debug/havi stamp.html -t 2 -o /tmp/final.mp4 -W 320 -H 240 2>&1 | tail -10 && ls -la /tmp/final.mp4`
Expected: progress bar shows, render completes, file non-empty.

Run: `./scripts/smoke_two.sh 2>&1 | grep -cE '"evt":"done"'`
Expected: `2`.

- [ ] **Step 5: Re-read every touched file, trim comments (CLAUDE.md STRIKE 7)**

Run: `grep -rn "// " src/host/ src/renderer/capture.rs src/renderer/paint.rs src/renderer/load.rs src/main.rs | grep -vE "Created by"`
Review each: keep only 1-line WHY for non-obvious constraints. Delete restating comments.

- [ ] **Step 6: Commit**

```bash
git add src/
git commit -m "refactor: unify all output on Evt wire format; CLI human-mode via emit_evt"
```

---

## Task 19: Update TypeScript types + docs

**Files:**
- Modify: `havi/index.d.ts`

API surface is unchanged, but document the new parallelism behavior.

- [ ] **Step 1: Add doc comment to render()**

In `havi/index.d.ts`, above the `render` signature in the `Havi` interface, add:

```typescript
export interface Havi {
  /**
   * Render HTML to video. Multiple concurrent calls run in parallel inside a
   * single shared host process (one CEF init, shared HTTP/GPU cache). Up to
   * HAVI_MAX_PARALLEL (default 4) renders execute at once; excess queue.
   */
  render(input: RenderInput): Promise<RenderResult>
  renderHelp(): string
}
```

- [ ] **Step 2: Verify dts matches the napi structs**

Run: `grep -nE "frames|width|height|fps|out|elapsedMs" havi/index.d.ts`
Expected: `RenderResult` fields match `src/napi/mod.rs` `RenderResult` (camelCase `elapsedMs`). napi-rs renames `elapsed_ms` → `elapsedMs` automatically. Confirm the d.ts already reflects that.

- [ ] **Step 3: Commit**

```bash
git add havi/index.d.ts
git commit -m "docs(types): note shared-host parallelism on render()"
```

---

## Task 20: Final verification + branch finish

**Files:** none

- [ ] **Step 1: Clean build from scratch**

Run: `cargo clean && cargo build --bin havi 2>&1 | tail -5 && cargo build --lib --features napi-binding 2>&1 | tail -5`
Expected: both clean, no warnings about unused code in touched modules.

- [ ] **Step 2: Run the full smoke suite**

Run:
```bash
./target/debug/havi stamp.html -t 2 -o /tmp/v1.mp4 -W 320 -H 240 2>&1 | tail -3
./scripts/smoke_two.sh 2>&1 | grep -cE '"evt":"done"'
```
Expected: one-shot produces /tmp/v1.mp4; smoke_two prints `2`.

- [ ] **Step 3: Verify no zombie processes after a kill**

Run:
```bash
./target/debug/havi stamp.html -t 30 -o /tmp/z.mp4 &
HPID=$!
sleep 3
kill -TERM $HPID
sleep 2
pgrep -fl "havi|ffmpeg" | grep -v grep || echo "no zombies"
```
Expected: `no zombies` (cleanup hooks + ffmpeg kill fire on SIGTERM).

- [ ] **Step 4: Confirm git state clean, summarize**

Run: `git log --oneline -20 && git status`
Expected: all tasks committed, working tree clean (smoke mp4s in /tmp, scripts/ committed).

- [ ] **Step 5: Invoke finishing-a-development-branch skill**

Hand off to `superpowers:finishing-a-development-branch` to decide merge/PR/cleanup.

---

## Self-Review

**Spec coverage:**
- One persistent host owns CEF → Tasks 9-11. ✓
- N concurrent renders, capped → Task 9 (`submit`/queue/`max_parallel`), Task 11 (env). ✓
- Shared RequestContext / cache → Task 11 (`root_cache_path` == `cache_path`), Task 17 (verify). ✓
- Cancel one render, others unaffected → Task 9 (`cancel`), Task 16 (test). ✓
- Backward-compatible JS API + CLI → Task 11 (CLI one-shot), Task 13 (napi same surface), Task 19 (dts). ✓
- IPC v2 Cmd/Evt one wire format → Task 1, Task 2, Task 18 (unify ipc::console/error onto Evt). ✓
- Per-render encoder → Task 8, wired Task 9. ✓
- Per-browser CDP routing → Task 4 (isolation proof), Task 9 (per-render observer). ✓
- Lifecycle (HostReady/HostExit/EOF/SIGTERM) → Task 10, Task 15, Task 20 (zombie check). ✓

**Placeholder scan:** No "TBD"/"implement later". Two spots flagged for signature confirmation (observer `Registration` return type in Task 9 Step 3; build.ts single-target invocation in Task 14 Step 1) — both include the exact command to discover the real signature and how to adapt. Acceptable: they're "run this to learn the exact type," not "figure it out."

**Type consistency:**
- `RenderId = u64` used everywhere (Task 1 defines, all tasks reference). ✓
- `Render` fields: defined Task 3, extended Task 9 (`encoder`, `errored`), `fake_render` test updated Task 9. ✓
- `Host::finish(id)` referenced in paint.rs (Task 6) + load.rs (Task 7), defined Task 9. ✓
- `Host::by_id` (Task 3) used in load.rs (Task 7). ✓
- `Host::dom_ready_advance` defined Task 11, called Task 11 main.rs. ✓
- `EncoderHandle` defined Task 8, used Task 9. ✓
- `emit_evt` defined Task 2, made human-aware Task 18, used Tasks 6/7/9. ✓
- `HostClient::begin/cancel/forget` defined Task 13, used Task 13 napi + Task 15 Drop. ✓

One correction applied during review: Task 13 `RenderResult` width/height/fps not in `Evt::Done` — resolved by capturing from opts before move (documented inline in Task 13 Step 2).
