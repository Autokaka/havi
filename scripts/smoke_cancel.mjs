import { havi } from "../havi/index.js";
const src = new URL("../examples/stamp.html", import.meta.url).pathname;
const ctrl = new AbortController();
const cancelled = havi.render({
  options: { source: src, out: "/tmp/c1.mp4", width: 320, height: 240, duration: 10 },
  signal: ctrl.signal, onConsole: () => {},
}).then(() => "RESOLVED(unexpected)").catch((e) => "rejected: " + e.message);
const survivor = havi.render({
  options: { source: src, out: "/tmp/c2.mp4", width: 320, height: 240, duration: 2 },
  onConsole: () => {},
});
setTimeout(() => ctrl.abort(), 600);
const [c, s] = await Promise.all([cancelled, survivor]);
console.log("cancelled:", c);
console.log("survivor:", s.frames, s.out, s.elapsedMs + "ms");
