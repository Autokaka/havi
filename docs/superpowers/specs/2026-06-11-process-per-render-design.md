# Process-per-render with isolated cache — design spec

**Date**: 2026-06-11
**Status**: approved (user pre-authorized), supersedes the multi-render host design
**Topic**: replace the persistent multi-render host with one isolated process per render

## Why this supersedes the host design

The multi-render host (one persistent process running N concurrent browsers, shared cache) solved concurrency **only for the napi path** — calls funnel through one shared process that serializes cache access. It did **not** fix the CLI path: each `havi <source>` invocation is an independent OS process, and N of them all point `root_cache_path` at the same `sandbox/profile` dir. Result, observed live: 3 concurrent CLI runs race on the SingletonLock (made worse by a lock-wipe hack), a CEF child process crashes, and a "something went wrong" dialog appears. Outputs may still land, but the cache is being corrupted and a process died.

The host also carried real cost: a stdin/stdout daemon protocol, browser-id routing, a per-render registry, a creating-render slot to survive `view_rect` during browser creation, a shared GPU process (one bad page can crash every in-flight render), and a long-lived fault domain. It is the opposite of simple.

First principles: a render is a pure function `(opts) → mp4 + event stream`. The simplest correct unit is **one OS process per render**. Make napi and CLI the *same thing* — a short-lived `havi` process. Isolation, not multiplexing, is what makes concurrency safe.

## Goals

1. Any number of concurrent renders — from CLI or napi — run without cache collision or crashes.
2. Each render owns an isolated cache dir, deleted when the render process exits.
3. No internal concurrency cap. The caller (business layer) decides how many run at once.
4. Within a render, a warmup pass (the existing phase-0 reload) warms that render's own cache before capture (pup-recorder pattern).
5. Drastically simpler codebase: no daemon, no host registry, no IPC routing.

## Non-goals

- Cross-render cache sharing. Each render is isolated; shared cache is what caused the collisions and bought little when render URLs differ.
- CEF-boot reuse across renders. Each process boots CEF once (~700ms). Accepted: a multi-second render amortizes it, and uniformity/simplicity/correctness outweigh it.
- A persistent daemon of any kind.

## Architecture

```
            napi havi.render(opts)                 CLI: havi <source> [opts]
                    │                                       │
                    ▼                                       ▼
       spawn `havi` subprocess (HAVI_IPC=1)        the same `havi` binary
                    │                                       │
                    └───────────────┬───────────────────────┘
                                    ▼
                       one havi process = one render
                       ├─ isolated cache dir (scratch/<pid>/cache)
                       ├─ CEF init (own SingletonLock, no contention)
                       ├─ one browser, warmup reload, capture loop
                       ├─ pipe BGRA frames → ffmpeg → mp4
                       └─ on exit: cleanup_session() removes the whole scratch dir
```

- **napi**: `render(opts)` spawns the `havi` binary with `HAVI_IPC=1`, reads JSON event lines (`Progress`/`Console`/`Done`/`Error`) on stdout, resolves the promise on `Done`. `AbortSignal` kills the child PID; the child's cleanup + parent-death watcher reap ffmpeg.
- **CLI**: identical binary, human output (progress bar + `done: ...`). No `--host`, no daemon.
- **Concurrency**: each render is its own process. 100 concurrent = 100 processes; the caller throttles. No queue, no cap inside havi.

## The one fix that makes it correct

`cef_init` uses an **isolated, per-process** `root_cache_path` instead of the shared `sandbox/profile`:

```rust
let cache_dir = scratch_dir().join("cache");   // scratch_dir = $TMPDIR/havi-<pid>-<rand>
settings.root_cache_path = CefString::from(cache_dir...);
```

- `scratch_dir()` is already unique per process and already removed by `cleanup_session()` on every exit path (normal, signal, panic, parent-death).
- Unique path ⇒ each process owns its own SingletonLock ⇒ zero contention ⇒ no crash dialog.
- The lock-wipe hack (`remove_file(SingletonLock/Cookie/Socket)`) is **deleted** — there is no shared lock to wipe.

## What gets deleted

- `src/host/` entirely (`render.rs` Host registry, `run.rs` daemon, `ipc.rs` Evt emitter, `mod.rs`).
- `HostClient` in `src/api.rs` → revert to the subprocess-spawn `RenderHandle`/`spawn` model.
- `Cmd`/`Evt` daemon wire types + `--host` CLI flag + `HAVI_MAX_PARALLEL`.
- Per-render browser-id routing in capture/paint/load → revert to a single per-process `State`.

## What is kept (orthogonal improvements)

- **Parent-death watcher** (`sandbox.rs`, getppid polling) — reaps a render process if its parent dies abruptly. Still valuable per-process.
- **Heartbeat keyframe** (`renderer/host.rs`) — keeps the compositor emitting BeginFrames.
- Determinism stack, encoder, proxy DSL, scheme handler — unchanged.

## Components after refactor

| File | Role |
|---|---|
| `main.rs` | wiring: parse Cli → cef_init (isolated cache) → one render → cleanup → exit |
| `renderer/capture.rs` | single `State` (per process), step/advance/stego, budget listener |
| `renderer/paint.rs` | `CaptureHandler` on one `State`, on_paint → ffmpeg |
| `renderer/load.rs` | phase machine on one `State` |
| `api.rs` | `RenderOpts` + `spawn(opts) → RenderHandle` (subprocess + event reader) |
| `napi/mod.rs` | `render()` via `api::spawn`; `AbortSignal` kills child |
| `common/ipc.rs` | `Msg` (Progress/Console/Done/Error) + human-mode progress bar |
| `common/sandbox.rs` | scratch/cleanup + parent-death watcher (kept) |

## Lifecycle (one process)

1. `havi <source>` (or napi-spawned with `HAVI_IPC=1`).
2. `cef_init` → isolated `scratch/<pid>/cache` as `root_cache_path`.
3. spawn ffmpeg, create browser, warmup reload (phase 0→1), prime (1→2).
4. capture loop: stego + virtual-time gate per frame → pipe to ffmpeg.
5. last frame → quit message loop.
6. `cleanup_session()` removes the scratch dir (cache + host page + frames). `shutdown()`. exit.
7. napi parent reads `Done`, resolves promise; or `AbortSignal` killed the process mid-flight (cleanup hooks + parent-death watcher reap ffmpeg).

## Risk register

| Risk | Mitigation |
|---|---|
| ~700ms CEF cold start per render | Accepted; amortized by multi-second renders; uniformity worth it |
| No shader-cache reuse across renders | Accepted; isolation is the safety property; shared GPUCache writes race |
| Caller spawns too many processes → machine overload | By design the caller throttles; document it; no internal cap |
| Orphaned scratch dirs on hard-kill (SIGKILL) | `scratch_dir` is under `$TMPDIR`, OS-reaped on reboot; cleanup hooks cover all soft exits + parent-death |

## Acceptance criteria

1. **3 concurrent CLI runs complete with zero crash dialogs** and 3 valid mp4s (the bug that triggered this redesign).
2. napi `Promise.all` of N renders all succeed (each its own process).
3. `AbortSignal` cancels one render; siblings unaffected.
4. SIGTERM on a render → no zombie havi/ffmpeg/helper processes.
5. Each render leaves no scratch dir behind after exit.
6. `cargo build` (bin + napi lib) green; unit tests pass.
7. `src/host/` gone; no `--host`, no `HAVI_MAX_PARALLEL`, no `Cmd`/`Evt`/`HostClient` in the tree.
```
