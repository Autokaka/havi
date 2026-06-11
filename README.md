# havi

Turn any webpage into a video. Deterministic, frame-perfect, offline.

havi renders HTML — animations, video, canvas, SVG, WebGL — to an MP4 by driving a real Chromium and capturing each frame against a virtual clock. The same input produces the same bytes every run, so it fits CI, golden-file tests, and automated content pipelines.

## Install

```sh
npm i @autokaka/havi
```

The package ships its own Chromium (CEF) and ffmpeg, so there is nothing else to install and no network needed at render time. Prebuilt for macOS arm64, Linux x64, Linux arm64, and Windows x64.

## Quick start

CLI:

```sh
npx havi https://example.com -o demo.mp4
```

Node or Bun:

```js
import { havi } from "@autokaka/havi";

const result = await havi.render({
  options: {
    source: "https://example.com",
    width: 1920,
    height: 1080,
    fps: 60,
    duration: 10,
    out: "demo.mp4",
  },
});

console.log(result.out, result.frames);
```

## How it works

Each render runs in its own short-lived `havi` process: it loads the page once to warm an isolated cache, reloads into that warm cache, then steps a pinned virtual clock frame by frame. Every frame is gated on a two-signal check (the browser's virtual-time budget plus an embedded per-frame timestamp), so frame N is pixel-identical across runs regardless of real-world timing, network jitter, or machine speed. Captured frames are piped straight to ffmpeg.

Because each render is a separate process with its own cache, you can run as many at once as you like — havi does not cap concurrency. Control how many run in parallel from your own code; each one is independent and cleans up after itself.

## CLI

```
havi <source> [options]

  -W, --width <px>       output width            (default 1920)
  -H, --height <px>      output height           (default 1080)
  -f, --fps <n>          frames per second       (default 30)
  -t, --duration <sec>   seconds to capture      (default 5)
  -o, --out <path>       output file             (default out.mp4)
      --tolerant         render partial DOM if the page is slow to load
      --proxy <json>     HTTP rewrite/block/mock rules
```

`<source>` accepts an http(s) URL, a local file path, or a `data:` URI.

## API

```ts
import { havi } from "@autokaka/havi";

const result = await havi.render({
  options: {
    source: string,        // URL, file path, or data: URI
    out?: string,          // default "out.mp4"
    width?: number,        // default 1920
    height?: number,       // default 1080
    fps?: number,          // default 30
    duration?: number,     // seconds, default 5
    tolerant?: boolean,    // render partial DOM on slow load
    proxy?: ProxyRule[],   // HTTP rewrite/block/mock
  },
  onProgress?: (e) => void,  // { frame, total }
  onConsole?: (e) => void,   // { level, source, message }
  signal?: AbortSignal,      // cancel this render
});
// result: { frames, width, height, fps, out, elapsedMs }
```

`havi.renderHelp()` returns the CLI help text.

Cancel a render with an `AbortSignal`:

```js
const controller = new AbortController();
setTimeout(() => controller.abort(), 5000);
await havi.render({ options: { source, out }, signal: controller.signal });
```

### Proxy rules

Rewrite, block, or mock requests during a render. First match wins.

```js
proxy: [
  { pattern: "https://api.public.com/**", to: "https://api.internal/" },
  { pattern: "**/*.gtag*", block: true },
  { pattern: "https://mock.api/**", status: 200, body: "{}",
    headers: { "content-type": "application/json" } },
]
```

## Output

MP4 (H.265/HEVC), web-ready with faststart. Transparent pages produce HEVC with an alpha channel (`yuva420p`), suitable for compositing.

## License

BSD 3-Clause. See [LICENSE](./LICENSE).
