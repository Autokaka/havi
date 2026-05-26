#!/usr/bin/env bun
// Ship a release. Assumes ./build.ts all has been run.
//   ./ship.ts            zip dist/ → gh release → npm publish
//   ./ship.ts --dry      zip only, skip gh release + npm publish

import { existsSync } from "node:fs";
import { readdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { zipSync } from "fflate";

const ROOT = dirname(fileURLToPath(import.meta.url));
process.chdir(ROOT);

const TARGETS = ["darwin-arm64", "linux-arm64", "linux-x64", "win32-x64"];
const pkg = JSON.parse(await readFile("package.json", "utf8"));
const tag = `v${pkg.version}`;
const dryRun = process.argv.includes("--dry");

for (const t of TARGETS) {
  if (!existsSync(join("dist", t))) die(`missing dist/${t} — run ./build.ts all first`);
}

const archives: string[] = [];
for (const t of TARGETS) {
  const archive = join("dist", `havi-${t}.zip`);
  await rm(archive, { force: true });
  console.error(`packing dist/${t} → ${archive}`);
  await packDir(join("dist", t), archive);
  archives.push(archive);
}

if (dryRun) {
  console.error(`dry-run: ${archives.length} archives at dist/`);
  process.exit(0);
}

console.error(`creating gh release ${tag}`);
await run(["gh", "release", "create", tag, ...archives, "--title", tag, "--notes", `Release ${tag}`]);

console.error("publishing to npm");
await run(["npm", "publish", "--access=public"]);

console.error("done");

async function packDir(dir: string, archive: string) {
  const tree: Record<string, Uint8Array> = {};
  await walk(dir, dir, tree);
  await writeFile(archive, zipSync(tree, { level: 6 }));
}

async function walk(root: string, dir: string, out: Record<string, Uint8Array>) {
  for (const e of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, e.name);
    if (e.isDirectory()) await walk(root, path, out);
    else if (e.isFile()) {
      const rel = path.slice(root.length + 1);
      out[rel] = new Uint8Array(await readFile(path));
    }
  }
}

async function run(args: string[]) {
  const proc = Bun.spawn(args, { stdio: ["inherit", "inherit", "inherit"] });
  const code = await proc.exited;
  if (code !== 0) throw new Error(`failed: ${args.join(" ")} → exit ${code}`);
}

function die(msg: string): never { console.error(msg); process.exit(1); }
