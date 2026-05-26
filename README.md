# havi

**Turn any webpage into a video. Deterministic, frame-perfect, offline.**

```sh
npm i havi
npx havi https://example.com -o demo.mp4
```

That's it. CSS animations, GSAP, video, canvas, SVG — all captured exactly the same way every run.

## Why havi

- **Frame-perfect** — virtual clock + per-frame sync. Frame N is always pixel-identical across runs.
- **Real browser** — full Chromium under the hood. If it renders in Chrome, it renders in havi.
- **Offline, no headless Chrome** — bundles its own CEF + ffmpeg. Works behind firewalls.
- **JS API or CLI** — script it from Node/Bun or drive from a terminal.
- **HEVC with alpha** — transparent video output for compositing pipelines.

## CLI

```sh
havi <source> [options]

  -W, --width <px>          1920
  -H, --height <px>         1080
  -f, --fps <n>             30
  -t, --duration <sec>      5
  -o, --out <path>          out.mp4
      --tolerant            ship partial DOM on slow loads
      --proxy <json>        HTTP rewrite rules
```

`<source>` accepts URLs, file paths, or `data:` URIs.

## API

```js
import { havi } from "havi";

const result = await havi.render({
  source: "https://example.com",
  width: 1920,
  height: 1080,
  fps: 60,
  duration: 10,
  out: "demo.mp4",
});
```

## Platforms

macOS arm64 · Linux x64 · Linux arm64 · Windows x64

---

Tech internals + architecture: see [CLAUDE.md](./CLAUDE.md).
