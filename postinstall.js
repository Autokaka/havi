#!/usr/bin/env node
import { chmod, mkdir, readdir, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { arch, platform } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { unzipSync } from "fflate";

const pkg = createRequire(import.meta.url)("./package.json");
const REPO = "Autokaka/havi";
const here = dirname(fileURLToPath(import.meta.url));
const SUPPORTED = new Set(["darwin-arm64", "linux-arm64", "linux-x64", "win32-x64"]);
const target = `${platform()}-${arch()}`;
if (!SUPPORTED.has(target)) {
  console.error(`havi: unsupported platform ${target}`);
  process.exit(1);
}

const dist = join(here, "dist", target);
if (existsSync(dist) && (await readdir(dist)).length > 0) {
  console.error(`havi: dist/${target} already exists, skip`);
  process.exit(0);
}

const url = `https://github.com/${REPO}/releases/download/v${pkg.version}/havi-${target}.zip`;
console.error(`havi: downloading ${url}`);
const res = await fetch(url, { redirect: "follow" });
if (!res.ok) {
  console.error(`havi: download failed ${res.status} ${res.statusText}`);
  process.exit(1);
}

const files = unzipSync(new Uint8Array(await res.arrayBuffer()));
await mkdir(dist, { recursive: true });

const execNames = new Set(["havi", "havi.exe", "havi-helper", "ffmpeg", "ffmpeg.exe"]);
const execExts = new Set([".dylib", ".so", ".dll", ".node"]);
const isExec = (base) => {
  const ext = base.includes(".") ? "." + base.split(".").pop() : "";
  return execNames.has(base) || execExts.has(ext) || base.startsWith("havi Helper");
};

await Promise.all(Object.entries(files).map(async ([name, content]) => {
  if (name.endsWith("/")) return;
  const out = join(dist, name);
  await mkdir(dirname(out), { recursive: true });
  await writeFile(out, content);
  const base = name.split("/").pop();
  if (isExec(base)) await chmod(out, 0o755).catch(() => {});
}));

console.error(`havi: installed to ${dist}`);
