# Process-per-render refactor — implementation plan

**Goal:** Replace the persistent multi-render host with one isolated `havi` process per render, each with its own cache dir (deleted on exit). Fixes the concurrent-CLI crash; deletes the daemon layer.

**Strategy:** The pre-host commit `b4bcc67` already *is* the one-process-per-render model — it only had the shared-cache bug. So: restore the host-touched files to `b4bcc67`, keep the two orthogonal improvements made since (parent-death watcher, heartbeat), delete `src/host/`, then apply the single fix (isolated per-process cache) + re-add the parent-death-watcher call.

---

## Step 1: Restore one-process files to b4bcc67

Restore exactly these (they are pure host-machinery reverts):

```bash
git checkout b4bcc67 -- \
  src/api.rs \
  src/cli.rs \
  src/common/ipc.rs \
  src/lib.rs \
  src/main.rs \
  src/napi/mod.rs \
  src/renderer/capture.rs \
  src/renderer/load.rs \
  src/renderer/paint.rs \
  src/video/encoder.rs
```

Do NOT restore (keep current — improvements):
- `src/common/sandbox.rs` (parent-death watcher)
- `src/renderer/host.rs` (heartbeat keyframe)
- `src/cef/cdp.rs` (added isolation test; harmless, single `State` uses one `Cdp`)

## Step 2: Delete the host module

```bash
git rm -r src/host
```

`src/lib.rs` was just restored to b4bcc67, which has no `pub mod host;` — confirm it's absent. If present, remove it.

## Step 3: Apply isolated per-process cache in main.rs cef_init

In the restored `src/main.rs`, replace the `cef_init` cache lines:

```rust
    // Default profile collides with stale CEF processes — pin to sandbox.
    let profile_dir = havi_core::sandbox_dir().join("profile");
    settings.root_cache_path = CefString::from(profile_dir.to_string_lossy().as_ref());
```

with an isolated per-process cache (unique path ⇒ own SingletonLock ⇒ no contention):

```rust
    // Isolated per-process cache — unique path means own SingletonLock, no
    // contention when many havi processes run at once. Removed on exit.
    let cache_dir = havi_core::scratch_dir().join("cache");
    let _ = std::fs::create_dir_all(&cache_dir);
    settings.root_cache_path = CefString::from(cache_dir.to_string_lossy().as_ref());
```

`scratch_dir()` is `$TMPDIR/havi-<pid>-<rand>`, already removed by `cleanup_session()` on every exit path. No lock-wipe needed.

## Step 4: Re-add the parent-death watcher call

The restored `b4bcc67` main.rs calls `havi_core::install_cleanup_hooks();` but not the watcher (it didn't exist then). Add the watcher call right after it:

```rust
    havi_core::install_cleanup_hooks();
    havi_core::install_parent_death_watcher();
```

`install_parent_death_watcher` is exported from `lib.rs` via `sandbox` (kept). Confirm `lib.rs` re-export includes it; b4bcc67 lib.rs may not export it. If the restored `lib.rs` `pub use sandbox::{...}` list omits `install_parent_death_watcher`, add it.

## Step 5: Build gate

```bash
cargo build --bin havi 2>&1 | tail -3              # Finished
cargo build --lib --features napi-binding 2>&1 | tail -3   # Finished
cargo test --lib 2>&1 | tail -6                    # pass (cdp test only; ipc Msg tests if any)
```

Fix any reference that still points at deleted host symbols.

## Step 6: Acceptance — rebuild bundle, run the crash repro

Rebuild release `havi` + `havi-helper` + `.node`, refresh `dist/darwin-arm64/havi.app`, re-sign.

1. **3 concurrent CLI (the repro):**
   ```
   havi stamp.html -t 2 -o /tmp/a.mp4 &   havi stamp.html -t 2 -o /tmp/b.mp4 &   havi stamp.html -t 2 -o /tmp/c.mp4 &
   ```
   Expect: 3 valid mp4s, exit 0, **no crash dialog**, three distinct `scratch/havi-<pid>` cache dirs that all vanish on exit.
2. **napi parallel:** `Promise.all` of 3 renders → 3 mp4s.
3. **cancel:** AbortSignal one of two → cancelled rejects, sibling completes.
4. **zombies:** SIGTERM a render → no leftover havi/ffmpeg/helper.
5. **no scratch leak:** after all exit, `ls $TMPDIR/havi-*` empty.

## Step 7: Docs + commit

- Update `havi/index.d.ts` render() doc: no shared host; each call is its own isolated process; caller throttles.
- Update `README` if it mentions the host/daemon.
- Commit. Update memory with the architecture decision.

## Notes for the implementer

- This is mostly `git checkout` + 2 small edits. Keep it surgical; do not hand-rewrite files that `git checkout` restores cleanly.
- After restore, re-read every touched file for stray host references (`grep -rn "host::" src/`, `grep -rn "HostClient\|Cmd::\|Evt::\|max_parallel\|--host" src/`). All must be gone.
- The b4bcc67 `napi/mod.rs` already has the AbortSignal + custom `FromNapiValue` + `RenderInput` design the user liked — restoring it brings that back intact.
