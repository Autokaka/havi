# Examples

Self-contained HTML pages for trying havi and as render test fixtures. None use remote assets, so they render fully offline and deterministically.

Render any of them:

```sh
havi examples/canvas-particles.html -t 4 -o particles.mp4
```

| File | Shows |
|---|---|
| `css-keyframes.html` | Pure CSS keyframe animation — orbiting, pulsing dots |
| `canvas-particles.html` | `requestAnimationFrame` canvas drawing — a particle swarm |
| `svg-spinner.html` | Animated SVG — rotating rings, `stroke-dasharray`, SMIL `<animate>` |
| `transparent-ring.html` | Transparent background — captured as HEVC with alpha for compositing |
| `typing.html` | Timed text reveal driven by the virtual clock |
| `audio-context.html` | `<audio>` + `AudioContext` analyser visualizer (in-memory WAV) |
| `stamp.html` | Determinism fixture — encodes the virtual timestamp each frame |

`scripts/smoke_tolerant.sh` renders every example in `--tolerant` mode and fails on any stall — a regression guard for the warmup-reload sequencing.

`transparent-ring.html` produces alpha video. Composite it over any backdrop; the page background stays transparent end to end.

To confirm determinism, render the same file twice and compare — the outputs are byte-identical.
