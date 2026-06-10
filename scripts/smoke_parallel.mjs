import { havi } from "../havi/index.js";
const src = new URL("../stamp.html", import.meta.url).pathname;
const mk = (out) => havi.render({
  options: { source: src, out, width: 320, height: 240, duration: 2 },
  onConsole: () => {},
});
const t0 = Date.now();
const [a, b, c] = await Promise.all([mk("/tmp/p1.mp4"), mk("/tmp/p2.mp4"), mk("/tmp/p3.mp4")]);
console.log(`ALL DONE in ${Date.now()-t0}ms`);
console.log("p1:", a.frames, a.out, a.elapsedMs+"ms");
console.log("p2:", b.frames, b.out, b.elapsedMs+"ms");
console.log("p3:", c.frames, c.out, c.elapsedMs+"ms");
