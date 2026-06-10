# Multi-render host process — design spec

**Date**: 2026-06-10
**Status**: draft, awaiting review
**Topic**: refactor `havi` from "1 process per render" to "1 persistent host process, N concurrent renders, shared CEF + cache"

## Problem

Today each `havi.render()` call from the napi binding spawns a fresh subprocess. Each subprocess re-initializes CEF (cold start ~700ms on M-series), claims the shared `root_cache_path`, and races other concurrent subprocesses for the SingletonLock. Symptoms:

- `SingletonLock` collisions cause the second concurrent render to fail or stall.
- Wiping the lock on startup (current workaround) avoids the collision but per CEF docs invites disk-cache corruption when multiple processes write the same `root_cache_path`.
- CEF cold start is paid per render. Cache reuse (HTTP, GPU shader, V8 code cache) is unreliable.
- Crashes leak ffmpeg subprocesses because cleanup is per-subprocess.

Electron sidesteps SingletonLock by simply not calling `requestSingleInstanceLock()`, which Chromium otherwise enforces. CEF 148 uses the Chrome runtime exclusively (Alloy removed) and always creates the SingletonLock; we cannot opt out the way Electron does.

The only docs-blessed path to safe concurrent rendering on CEF is: **one process owns CEF, hosts N concurrent browsers, each browser is one render, all browsers share one `RequestContext`**. That is the design here.

## Goals

1. One persistent host process owns CEF for the lifetime of the napi module instance.
2. Multiple concurrent `havi.render()` calls execute in parallel inside the same host, capped by a configurable concurrency limit.
3. HTTP cache, cookies, and GPU shader cache are shared across renders within the host process.
4. Cancellation of one render must not affect others.
5. Existing JS API and CLI surface stay backward-compatible — no breaking changes for users.

## Non-goals

- Cross-host-process cache sharing. With one host, there is nothing to share across.
- CLI batch mode (`havi --batch jobs.jsonl`). CLI stays one-shot. Multi-render is napi-only.
- Hot-reload of CEF within the host. CEF init is one-time.

## Architecture

```
+---------------------+   stdio JSON lines    +-----------------------------+
| napi module         | <-------------------> | havi --host (subprocess)    |
|                     |                       |                             |
| HostHandle (lazy)   |   cmd: start/cancel   | CEF (initialized once)      |
|   - child process   |   cmd: shutdown       |                             |
|   - stdin Mutex     |                       | RequestContext (shared)     |
|   - reader thread   |   evt: started        |                             |
|   - pending map     |   evt: progress       | Host                        |
|     (id -> chan)    |   evt: console        |   renders: {                |
|                     |   evt: done | error   |     id -> Render { ... }    |
| render(input):      |   evt: host_ready     |   }                         |
|   - alloc id        |   evt: host_exit      |   by_browser: { ... }       |
|   - send start      |                       |                             |
|   - await done      |                       | Browser #1 (Render id=1)    |
|   - on abort: cancel|                       | Browser #2 (Render id=2)    |
+---------------------+                       | ...                         |
                                              +-----------------------------+
```

CLI mode (`havi <source>`) is implemented as `--host` mode internally: same process initializes a host, submits a single start command, and exits when that render finishes. No duplicate code path.

## Per-render state

Each in-flight render owns one `Render` struct, keyed by a `u64` ID allocated by the caller (napi assigns from its own counter; CLI uses ID 0):

```rust
pub type RenderId = u64;

pub struct Render {
    pub id: RenderId,
    pub opts: RenderOpts,
    pub browser: BrowserHandle,           // Arc<Mutex<Option<Browser>>>
    pub iframe: FrameHandle,              // Arc<Mutex<Option<Frame>>>
    pub cdp: Cdp,                         // per-render CDP wrapper
    pub capture: CaptureState,            // next_frame, requested_ms, stuck_invalidates, budget_done
    pub encoder_pid: u32,
    pub encoder_tx: Option<Sender<Frame>>,
    pub scratch: PathBuf,                 // per-render scratch root (host.html, temp ffmpeg PNGs)
    pub phase: u8,
    pub started_at: Instant,
    pub total_frames: u32,
    pub frame_ms: f64,
}

pub struct Host {
    pub renders: Mutex<HashMap<RenderId, Arc<Mutex<Render>>>>,
    pub by_browser: Mutex<HashMap<i32, RenderId>>,  // CefBrowser::identifier() -> RenderId
    pub request_context: RequestContext,            // one shared context
    pub max_parallel: usize,
    pub queue: Mutex<VecDeque<PendingStart>>,       // overflow when at capacity
}
```

Routing CEF callbacks to the right render: handlers receive `&Browser`; look up `browser.identifier()` in `by_browser` to find the `RenderId`, then fetch the `Render` from `renders`.

## CDP per browser

CDP send is already keyed by `BrowserHost` (we pass `&host` to `send`). The only changes:

- Each render has its own `Cdp` instance. The internal request-ID counter can stay process-wide (uniqueness is fine across browsers; observers only see their own browser's replies).
- Each render registers its own `CdpObserver` against its browser, carrying the `RenderId`. Events route directly to that render's state without map lookups.

## Encoder per render

Move the ffmpeg spawn out of `main.rs` and into `Render::start`. Each render owns:

- its own ffmpeg child PID,
- its own BGRA frame `Sender`,
- a join handle for the encoder pump thread.

The global `pids()` set (used for signal cleanup) keeps tracking all ffmpeg PIDs across all renders so SIGTERM still nukes everything.

On render done or cancel: drop the `Sender`, join the pump thread, `wait()` the ffmpeg child, call `unregister_ffmpeg(pid)`.

## Shared RequestContext

One `RequestContext` is created at host init using the global default settings (`root_cache_path` and `cache_path` both set to `sandbox_dir().join("profile")`). All browsers are created with this context. This is the configuration CEF docs identify as "share the global browser cache and related configuration".

`root_cache_path` is stable across host restarts. SingletonLock is held by exactly one process — the host. No wipe-on-startup hack needed.

## IPC v2 protocol

Line-delimited JSON on host stdio. stderr is unchanged (CEF/ffmpeg noise still goes there). stdout is reserved for IPC.

Wire types defined once in `src/common/ipc.rs`, used by both host (emits Evt, parses Cmd) and napi (parses Evt, emits Cmd):

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Cmd {
    Start { id: RenderId, opts: RenderOpts },
    Cancel { id: RenderId },
    Shutdown,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "evt", rename_all = "snake_case")]
pub enum Evt {
    HostReady,
    Started   { id: RenderId },
    Progress  { id: RenderId, frame: u32, total: u32 },
    Console   { id: RenderId, level: String, source: String, message: String },
    Done      { id: RenderId, out: String, frames: u32, elapsed_ms: u64 },
    Error     { id: RenderId, message: String },
    HostExit,
}
```

Lifecycle invariants:

- Host emits `HostReady` once after CEF init. Parent waits for this before sending the first `Start`.
- Every `Start` is acknowledged by exactly one of `Done` or `Error` for that ID.
- `Cancel` for an in-flight render produces `Error { message: "cancelled" }` for that ID.
- `Shutdown` waits for in-flight renders to finish, emits `HostExit`, then exits.
- Stdin EOF is treated as implicit `Shutdown`.
- SIGTERM triggers immediate cleanup (kill ffmpeg subprocesses, close browsers) and exit without waiting.

## Concurrency control

`Host::max_parallel` caps how many renders execute concurrently. Excess `Start` commands queue. When a render finishes, the queue is drained by one.

**Decision**: default cap is `4`. Override via env `HAVI_MAX_PARALLEL`. Rationale: 4 concurrent OSR browsers at 1920×1080 BGRA ~ 4 × 1920 × 1080 × 4 ≈ 32 MB per backing store plus per-renderer overhead (~150 MB each). On a base 16 GB Mac that leaves headroom; users with larger budgets bump the env var.

## Cancellation semantics

**Decision**: cancel sends SIGTERM to ffmpeg with 1-second grace, then SIGKILL. Browser is closed via `close_browser(true)` synchronously. Reasoning: ffmpeg flushes its mp4 trailer on SIGTERM, leaving a playable (if short) output file useful for debugging. The 1s ceiling caps wait time.

Cancel does NOT delete the partial mp4. Caller may discard.

## Crash and failure handling

**Decision**: panics and fatal errors write to stderr only — no `~/.havi/crashes/` directory. The napi parent reroutes host stderr to the user's terminal or wherever the Node process's stderr goes. Simpler, no orphan files.

Host crash detection (napi side):
- Reader thread sees stdout EOF without preceding `HostExit`.
- All pending render channels receive an `Error { message: "host died" }`.
- The `HostHandle` is reset (`OnceLock` is taken, replaced on next `render()` call).

GPU process crash inside host: CEF auto-restarts GPU. One frame may glitch on the affected render. Acceptable.

## Lifecycle

```
napi module load
    |
    v
first render() call ----> spawn `havi --host` subprocess
    |                          |
    v                          v
register render id        CEF init
write Start cmd                |
    |                          v
    |                     emit HostReady
    |                          |
    |<------ HostReady --------+
    |                          |
    | send Start cmd --------->|
    |                          v
    |                     create Browser
    |                     run render loop
    |                          |
    |<------ Progress* --------+
    |<------ Done -------------+
    v
resolve render() Promise

(many more render() calls reuse the same host)

napi module unload / Node exit
    |
    v
HostHandle drop ----> write Shutdown
    |                          |
    |<------ HostExit ---------+
    v
host child exit
```

## Phases

1. **Per-render state lift** — introduce `Render` struct, `Host` container, route CEF callbacks by browser ID. Single render still works end-to-end. ~200 LOC.
2. **Per-browser CDP** — each render owns its `Cdp` + `CdpObserver`. Verify by smoke-testing two parallel renders inside one process. ~80 LOC.
3. **Per-render encoder** — move ffmpeg ownership into `Render`. Two parallel renders produce two valid mp4s. ~30 LOC.
4. **Shared RequestContext** — create once at host init, pass to all `browser_host_create_browser_sync` calls. Verify cache reuse via cold/warm timing. ~20 LOC.
5. **`havi --host` daemon mode** — stdin command reader, stdout event emitter, `Cmd`/`Evt` enums in `ipc.rs`. ~150 LOC.
6. **napi rewires to host** — `OnceLock<HostHandle>`, reader thread fan-out, `Promise.all([render, render])` finishes faster than serial. ~100 LOC.
7. **CLI on top of host** — `havi <source>` is internally `--host + Start(0, opts) + drive until Done`. Old direct-render code path deleted. ~30 LOC, plus deletes.

Net change: ~600 LOC added, ~400 moved/deleted.

## Risk register

| Risk | Likelihood | Mitigation |
|---|---|---|
| CEF UI thread serialization limits effective parallelism | medium | UI thread mostly dispatches; renderer/GPU processes parallelize. Profile after phase 4; if it bottlenecks, revisit `multi_threaded_message_loop` on Linux/Windows (macOS does not support it). |
| GPU memory exhaustion at high parallel cap | low at default 4 | Document `HAVI_MAX_PARALLEL`; default 4 keeps memory in check on 16 GB machines. |
| One bad page crashes shared GPU process | low | CEF auto-recovers; one frame glitch acceptable. |
| Host dies mid-render → napi promises hang | mitigated | Reader thread detects stdout EOF → fail all pending renders. |
| Stamp.html stuck-at-frame-216 bug (current open issue) | unrelated | Independent fix via CDP reply-aware send; tracked separately. |
| RequestContext settings conflict (e.g., proxy rules differ per render) | medium | Proxy rules are applied at `RequestHandler` level (per-browser), not on RequestContext. No conflict. |

## Test plan

- Phase 1: existing single-render hash-stable test passes.
- Phase 2: in-process unit test creates two mock CDP observers, fires events from two browser IDs, asserts no cross-talk.
- Phase 3: integration test runs two `Render::start` instances on the same host (no daemon yet), checks both mp4 outputs exist and have correct frame counts.
- Phase 4: render the same URL twice in one host; assert second render's total time is at least 20% shorter (cache hit signal).
- Phase 5: scripted test pipes a JSONL command file into `havi --host`, parses event JSONL, asserts protocol invariants.
- Phase 6: JS test `Promise.all([havi.render(a), havi.render(b)])` finishes faster than the serial sum, both outputs valid.
- Phase 7: `havi sample.html` produces output identical to the phase-1 baseline.

## Decisions resolved

- **Concurrency cap**: default 4, env override `HAVI_MAX_PARALLEL`.
- **Cancellation**: SIGTERM ffmpeg + 1s grace + SIGKILL; browser closed synchronously; partial mp4 retained.
- **Crash dump location**: stderr only; no on-disk crash directory.
- **CLI multi-source**: not in scope; CLI stays one-shot.
- **Host persistence across Node worker_threads**: each Node process gets its own host; not shared across workers.

## Open questions

None at design time. All decisions resolved above. Re-open during implementation only if a phase's exit criterion cannot be met.
