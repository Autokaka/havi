# havi — project rules

Style conventions for Claude when working in this repo.

## Tone

User uses caveman mode. Replies: terse, fragments OK, drop articles/filler/
hedging. Code, commits, security text: write normal.

## Comments (STRICT — STRIKE 7)

Default: **NO comment**. Code self-documents via good names.

Allowed (rare):
- One-line WHY, max ~80 chars
- Only when behavior surprises (hidden constraint / non-obvious workaround / known gotcha)

HARD BANNED — no exceptions:
- Multi-line comment blocks (more than 1 line)
- Docstrings on fns/structs
- Module-level "Module map" / "Owns" / "Layer" prose
- Restating what the code does ("Set X to Y" above `x = y;`)
- Bullet lists explaining obvious
- Banner section comments (`// ---- foo ----`)
- Comments that reference current task/PR/issue numbers
- Justifying-the-line-by-explaining-the-line comments

Pre-write checklist before adding ANY comment:
1. Is this a 1-line WHY? If no → drop.
2. Would removing the comment confuse a future reader? If no → drop.
3. Does the name/structure already imply this? If yes → drop.

After every refactor: re-read every touched file and trim. Burned 7 times.

## Rust idiom (STRICT — write it like a Rustacean or don't write it)

Ridiculous code gets reverted on sight. Before you write ANY Rust here, the
default is the simplest ownership that works. Reach for a sync primitive or an
allocation only when you can name why nothing cheaper suffices.

HARD RULES — no exceptions:
- **No `Arc<Mutex<T>>` reflex.** If state is written by one owner and read after
  it finishes → return it (thread `JoinHandle<T>`, channel, or plain move). If
  it's truly shared across threads/callbacks → fine. A `Mutex` whose value is
  read only once the writer is done is a code smell. Prove the sharing is real.
- **No allocation theatre.** Don't build `Vec<String>` of flags when `&[&str]`
  + `cmd.args()` works. Don't `.to_string()`/`.clone()` to dodge a borrow. Don't
  `format!` where a `&str` literal does. Clone `Arc` freely (cheap); clone owned
  data only when ownership genuinely must split.
- **No panic on the outside world.** `.unwrap()`/`.expect()` ONLY on: lock
  poisoning, `OnceLock`/invariants you just established, or startup asserts.
  NEVER on I/O, subprocess exit, network, parses of external/CLI/FFI data, or a
  channel that a peer thread can drop. Those return `Result`/`Option` and are
  handled — a dead subprocess must surface an error, not abort the process.
- **`TryFrom`, not `as`** for anything lossy/narrowing on external data (CLI,
  FFI, ffmpeg, file sizes). `as` is allowed ONLY for lossless widening
  (u32→i64, u8→u32) and pointer/FFI casts. `value as u8` on untrusted width is
  banned (principle 12).
- **Std over hand-roll.** No reverse-then-reverse, no char-by-char loop where an
  iterator adapter / `split_once` / `trim_matches` / `find`+slice reads clearer.
- **Enums over stringly-typed** internal state. Pass `Format`, not `"webm"`,
  across Rust boundaries (CLI flag strings at the edge are fine).

Pre-write gut check: "Is there a simpler ownership/type that a senior Rust dev
would reach for first?" If yes, write that. After every edit, re-read for
`Arc<Mutex`, stray `.clone()`/`.to_string()`, and `.expect()` on I/O — trim them.

## File naming

- Rust source: snake_case (`my_module.rs`)
- Rust binary names: kebab-case in Cargo.toml (`havi-helper`)
- Build scripts: `make.rs` (rust-script), NOT `build.rs` at root — Cargo
  reserves that name
- JS files: snake_case, grouped under `src/runtime/`
- Sibling distrib JS to bundle: `havi.mjs` (consumer-facing), `index.mjs`
  (napi loader). User runs via `bun havi.mjs` etc.

## Module layout

Hard cap: **~150 LOC per module**. Split when over.  
**Exception (principle 11)**: don't split trivial helpers tightly coupled to
one flow just to hit the cap. Split-by-concern > split-by-size.

`main.rs` is wiring-only — orchestration, no business logic. Helpers in
main.rs are OK if they're inline-call wiring (`cef_init`, `report_done`).

## Build orchestration

- `./make.rs` — single rust-script orchestrator at project root. Idempotent.
  Handles: cargo build, bundle (macOS), sidecar ffmpeg (all platforms),
  codesign, sign cert creation, llvm-strip libcef.so.
- `cargo install rust-script` is the only prerequisite the script can't
  bootstrap.
- Outputs land in `dist/<plat>-<arch>/`. `target/` keeps cargo intermediates
  only. Repo root stays clean.
- No `build.rs` at repo root. ffmpeg ships as a sidecar binary next to havi
  on every platform.

## Cross-platform

| Concern | macOS arm64 | Linux | Windows x64 |
|---|---|---|---|
| Layout | `.app` bundle | single binary | single .exe |
| Triple | `aarch64-apple-darwin` (x64 dropped, macOS 27+) | arm64 + x64 | `x86_64-pc-windows-msvc` only — arm64 disabled (xwin ARM64EC + LLVM 22 lld-link bug) |
| Codesign | stable self-signed (`havi-codesign`) via `make.rs` | none | none |
| ffmpeg | sidecar in `Contents/MacOS/ffmpeg` (signed with bundle) | sidecar next to `havi` | sidecar next to `havi.exe` |
| GPU backend | `use-angle=metal` | `use-angle=vulkan` | `use-angle=d3d11` |
| Sandbox dir | `havi.app/Contents/MacOS/sandbox/` | `<binary-dir>/sandbox/` | `<binary-dir>\sandbox\` |
| Parent-death signal | none (kernel lacks; explicit kill required) | `prctl(PR_SET_PDEATHSIG)` | none (taskkill fallback) |
| libcef.so strip flag | `llvm-strip -x` (Mach-O) | `llvm-strip --strip-unneeded` (ELF) | same as Linux (PE) |

All scratch state (CEF cache `sandbox/profile/`, ffmpeg frame PNGs, stego
host page) under one sandbox root next to the havi binary. Single dir to
nuke.

CEF distribution = spotify-cdn `_minimal` variant (`archive.json type:
minimal`). Still ships unstripped libcef.so (~1.9 GB on arm64). make.rs
strips via llvm-strip post-copy. Reduces dist ~85%.

## Refactor principles (learned & enforced)

1. **No hand-rolled JSON / serializers** — use `serde` + `serde_json` with
   `#[serde(tag = "type", rename_all = "lowercase")]` for tagged-union wire
   formats. One trait derive replaces tens of LOC of brittle parsing.
2. **One wire format, two consumers** — when a value is emitted on one side
   and parsed on the other (IPC, RPC, file format), define the type ONCE
   with `Serialize + Deserialize`. Never write parallel reader/writer
   structs.
3. **Std + canonical crates over hand-roll** — `signal-hook` for signals
   (libc alone can't safely malloc/lock inside signal handler), `serde_json`
   over hand-roll, `clap` for CLI, `globset` for URL pattern matching.
4. **Persistent vs ephemeral state** — split sandbox layout: persistent
   (CEF profile, expensive shader caches) lives in `sandbox/` sibling of
   binary; ephemeral (per-render frames, host page) in
   `$TMPDIR/havi-<pid>-<rand>`, OS reaps on reboot, cleanup hooks on every
   exit path.
5. **Feature-gated FFI** — when one crate is both Rust lib + napi binding,
   make `napi`/`napi-derive` optional behind a `napi-binding` feature.
   Build bin without feature, cdylib with. Avoids "node symbols not found"
   link errors when bin tries to consume the rlib.
6. **Self-locating cdylib** — `dladdr` (Unix) / `GetModuleHandleEx` (Win)
   returns the loaded library's path. Lets a `.node` file find its sibling
   CLI binary with zero JS-side configuration, zero env vars.
7. **First-principles audit order** — (a) kill duplication, (b) split big
   files, (c) inline tiny helpers. Don't refactor in reverse: trimming
   before deduping just moves the dup around.
8. **One struct, conditional decorators** — `#[cfg_attr(feature = "x",
   napi(object))]` on a single type lets Rust API users + napi-binding
   share one definition. Parallel `api::RenderOpts` and `napi::RenderOpts`
   is rubbish; collapse.
9. **Type alias repeated handle patterns** — `Arc<Mutex<Option<T>>>` shown
   in 5+ sites becomes `BrowserHandle` (alias at type origin). Reads
   clearer, refactors easier.
10. **Don't force-merge structs with conflicting type models** — `Cli`
    uses clap default-value-t (concrete defaults); `RenderOpts` uses
    Option<T> (caller-omits). Different shapes → keep separate.
11. **Split-by-concern beats split-by-size** — when a 200-LOC file is one
    Session struct + a 100-LOC `decode_thread()` function, lift the
    function into a sibling `decode.rs`. Don't extract trivial helpers
    just to hit the LOC cap.
12. **Casts via TryFrom, not `as`** — `as` only for lossless widening
    (u32→f64, u8→i32). Everywhere else (i32→u32, usize→i64, u32→i32,
    u128→u64) use `TryFrom::try_from(...).unwrap_or(default)` or
    `i64::from(val)` for widening. Use `checked_mul` for pointer-math on
    untrusted dimensions. `as` casts on data from FFI / CLI / external is
    a silent overflow trap.
13. **Resource-handler dedup pattern** — one ResourceHandler struct (e.g.
    `SyntheticResource` with optional headers map) serves multiple
    consumers. Factories like `make_handler(...)` /
    `make_handler_with_headers(...)`. Routes delegate via one fn per
    path. Don't write parallel `FrameHandler` + `ProxyResponse`.
14. **Single source of truth for DOM observation** — when both an IDL
    setter override AND a MutationObserver fire `onSrcChange` for the same
    src mutation, dedupe via state-tracking (`state.attachedSrc == newSrc`
    → no-op), OR drop one path. Default: drop the setter override; MO
    catches programmatic + setAttribute uniformly.
15. **Native getters lie after method override** — after overriding
    `HVE.play` / `HVE.pause`, the native `video.paused` flag is stale (our
    override bypasses native algorithm). Track explicit intent on the
    element (`video.__havi_play_intent = true/false`) and consult that in
    attach/state-init, not native paused.
16. **Subprocess set, not single handle** — `Mutex<HashSet<i32>>` for
    tracking ffmpeg PIDs (encoder + per-video decoders). Register on
    spawn, unregister on `wait()`, kill all on signal. `std::process::Child`
    does NOT kill on Drop, and `panic=abort` + `process::exit` skip Drop
    anyway. Without explicit tracking, subprocesses leak on every abnormal
    exit.

## Hard-won implementation rules

**CEF command-line value switches**:
`append_switch("key=value")` stores the switch under key `"key=value"` with an
empty value — Chromium's `GetSwitchValueASCII("key")` then never finds it, so
the flag silently no-ops (only "works" when its value matches the default).
For any `key=value` flag (autoplay-policy, force-color-profile,
force-device-scale-factor, use-angle, disable-features, …) split on the first
`=` and use `append_switch_with_value(key, value)`. Fixed in `cef/app.rs`.

**CEF platform-typed integers**:
`LogSeverity::get_raw()` and similar return `u32` on macOS, `i32` on
Windows. Compare via `as i64` cast for cross-platform builds.
`cargo build` on host succeeds; xbuild for Windows fails. Fix once in
`cef/client.rs::map_level`.

**file:// URL normalization** (in `renderer/host.rs::normalize_source`):
- `file:////Users/...` (4+ slashes) → ffmpeg confused, iframe base
  ambiguous
- Bare path → unresolved against scratch_dir, not user CWD
- Canonical output: always `file:///<absolute-path>`
- Branch: `data:` / `://` pass through; `file:` prefix → strip + collapse
  slashes + `file:///`; bare → canonicalize + `file:///`.

**Mach-O strip flag**:
`llvm-strip --strip-unneeded` works for ELF/PE; errors out on Mach-O.
Use `-x` for Mach-O (strips local symbols). Branch in
`make.rs::strip_release` by file extension.

**Bundle helpers refresh**:
macOS `havi Helper.app`, `havi Helper (GPU).app`,
`havi Helper (Renderer).app` are renamed copies of `havi-helper` binary.
After rebuild: copy `target/release/havi-helper` to ALL THREE, then
`codesign --force --sign - --deep` the whole bundle. `make.rs` handles
this; ad-hoc rebuilds during dev must too.

**Console-capture phasing** (`ipc::set_console_capture(true)` at phase 0→1):
First-load iframe `console.log` calls are SUPPRESSED — debug logging in
iframe_hook initialization won't appear in stderr. Verify hook behavior
via post-warmup logs (after first `load_end`).

**bundle-cef-app cosmetic noise**:
`bundle-cef-app` runs its own `cargo build` (debug profile) during
bundle scaffolding. Output appears in build logs. Subsequent
`lipo`/copy in make.rs overwrites with release binaries. Not a real
problem, just noisy.

## Subprocess lifecycle (cancellation)

ffmpeg subprocesses (one encoder + N decoders, one per video element).
Goal: process + all children + scratch dirs cleaned up on any abnormal
exit. Non-healthy output mp4 acceptable.

Mechanism:
- `signal-hook` (SIGINT/TERM/HUP/QUIT) → side thread → `cleanup_session()`
  → `kill_all_ffmpeg()` + `remove_dir_all(scratch_dir)` → `exit(128+sig)`.
- `panic::set_hook` → same path before abort.
- Linux: `pre_exec(|| libc::prctl(PR_SET_PDEATHSIG, SIGTERM))` in
  encoder + decoder spawn. Kernel-mediated kill if parent dies abruptly
  (SIGKILL, OOM, segfault).
- macOS: no kernel parent-death; relies on signal handler. SIGKILL on
  parent leaks ffmpeg children — known limitation.
- Windows: `taskkill /F /PID <pid>` shell-out (avoids `windows-sys`
  Job Object dep for one feature). Adequate.

PID tracking: `sandbox::register_ffmpeg(pid)` on spawn,
`unregister_ffmpeg(pid)` after `wait()`. Kill iterates the set; clears
after kill.

## Determinism stack

1. CDP `Emulation.setVirtualTimePolicy` (master clock; `initialVirtualTime`
   pinned to 2020-01-01 UTC)
2. iframe HOOK injected at V8 context create (`src/runtime/iframe_hook.js`):
   overrides Date, performance.now, rAF queue, timers, Math.random,
   `document.getAnimations()`
3. Stego host page wraps user URL in iframe; bottom 1-pixel row encodes
   virtual timestamp; on_paint decodes and drops paints whose stego ≠
   requested ms
4. Heartbeat keyframes — 1×1 CSS animation keeps compositor emitting
   BeginFrames on static pages

Two-signal gate per frame: `virtualTimeBudgetExpired` event + matching stego.

## Codec / encoder

- HEVC alpha via `-c:v libx265 -pix_fmt yuva420p -x265-params alpha=1`
  cross-platform (bit-exact). VideoToolbox encoder dropped — alpha quality
  varies, not deterministic.
- Container: `.mp4` with `-tag:v hvc1`, `-movflags +faststart` for web playback
- Encoder preset: `fast` + `-crf 23` + multi-thread (`pools`/`frame-threads`)
- ffmpeg = jellyfin-ffmpeg 8.1 (downloaded from `seydx/node-av` releases) —
  default Apple/brew builds lack `x265 ENABLE_ALPHA`
- Transparent OSR: `browser_settings.background_color = 0` + host page
  `background: transparent`

## HEVC-alpha decode guardrail (HARD BAN)

NEVER use `-hwaccel auto` (or any specific HW decoder: videotoolbox, cuvid,
d3d11va, vaapi) for video decode in this project. HW decoders silently strip
the alpha plane (yuva420p → yuv420p) without erroring. Output looks fine
visually but transparency is lost. Always use SW decode (default, no
-hwaccel flag) in the `havi-frame://` pipeline.

## CLI output

- Default: clean. `rendering ... → out.mp4` + indicatif progress bar +
  `done: ... in Ns`. Bar uses `.println()` for non-disruptive log layout.
- CEF logs disabled (`settings.log_severity = LogSeverity::DISABLE`)
- ffmpeg `-loglevel error`
- Errors prefixed with `\n` so they don't smear the live progress line
- No per-frame eprintlns
- IPC mode (env `HAVI_IPC=1`): emits JSON lines to stdout for napi
  subprocess parent to read

## HTTP proxy DSL

CLI: `--proxy '<json-array>'`. napi: `RenderOpts.proxy: ProxyRule[]`.
Same `ProxyRule` struct via `cfg_attr(napi(object))`.

```json
[{"pattern":"https://api.public.com/**","to":"https://api.internal/"},
 {"pattern":"**/*.gtag*","block":true},
 {"pattern":"https://mock.api/**","status":200,"body":"{}",
  "headers":{"content-type":"application/json"}}]
```

- `globset` compiles rules to single GlobSet (microsecond match)
- First-match-wins
- Hook on `RequestHandler::resource_request_handler` →
  `ResourceRequestHandler::on_before_resource_load` (rewrite/block) +
  `resource_handler` (mock response via SyntheticResource)

## Persistent-memory advisory

The `PUA Integrity Guard` hook may flag edits to long-term memory or this
CLAUDE.md as a governance risk. Edits are fine when they reflect actual
project state changes; just note the rationale in the response and proceed.
