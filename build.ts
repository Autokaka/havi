#!/usr/bin/env bun
// Build distributables from a macOS host.
//   ./build.ts                       current host (darwin-arm64), release
//   ./build.ts all                   every supported target
//   ./build.ts <target>              one target (darwin-arm64 | linux-arm64 | linux-x64 | win32-x64)
//   ./build.ts <target|all> debug    debug profile
//   ./build.ts debug                 current host, debug
// Always refreshes deps to latest before building (best-effort).
// CEF + ffmpeg are pinned binaries — bump their constants by hand.
// Outputs: dist/<platform>-<arch>/

import { $ } from "bun";
import { unzipSync } from "fflate";
import { existsSync, type Dirent } from "node:fs";
import { chmod, cp, mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { homedir, platform } from "node:os";
import { basename, dirname, extname, join } from "node:path";
import { fileURLToPath } from "node:url";

const APP = "havi";
const LIB = "havi_core";
const IDENT = "havi-codesign";
const CEF_CRATE_VERSION = "148.2.0";
const NODE_AV_TAG = "v5.2.4";
const FFMPEG_VER = "v8.1";

type Target = { tag: string; triple: string; plat: string; arch: string };

// macOS 27 drops x86_64. arm64 only.
// Windows arm64 disabled: xwin ARM64EC libs crash lld-link (LLVM 22 bug).
const TARGETS: Target[] = [
  { tag: "darwin-arm64", triple: "aarch64-apple-darwin", plat: "darwin", arch: "arm64" },
  { tag: "linux-arm64", triple: "aarch64-unknown-linux-gnu", plat: "linux", arch: "arm64" },
  { tag: "linux-x64", triple: "x86_64-unknown-linux-gnu", plat: "linux", arch: "x64" },
  { tag: "win32-x64", triple: "x86_64-pc-windows-msvc", plat: "win32", arch: "x64" },
];
const HOST_TARGET = TARGETS[0];
const PRUNED_HELPERS = ["havi Helper (Plugin).app", "havi Helper (Alerts).app"];
const FFMPEG_PLAT: Record<string, string> = { darwin: "macos", linux: "linux", win32: "win" };

const ROOT = dirname(fileURLToPath(import.meta.url));
process.chdir(ROOT);
if (platform() !== "darwin") {
  die("macOS host required");
}

const { targets, profile } = parseArgs();
await ensureTools();
await updateDeps();
await prefetchFfmpegs(targets);

const outputs: Array<[string, string]> = [];
for (const t of targets) {
  const path = t === HOST_TARGET ? await buildMacos(t, profile) : await buildUnbundled(t, profile);
  outputs.push([t.tag, path]);
}

console.log(`\nprofile: ${profile}`);
for (const [tag, path] of outputs) console.log(`  ${tag.padEnd(13)}  ${path}`);

function parseArgs(): { targets: Target[]; profile: "release" | "debug" } {
  let chosen: Target[] | null = null;
  let profile: "release" | "debug" = "release";
  for (const a of process.argv.slice(2)) {
    if (a === "release" || a === "debug") {
      profile = a;
      continue;
    }
    if (a === "all") {
      chosen = [...TARGETS];
      continue;
    }
    const t = TARGETS.find((x) => x.tag === a);
    if (!t) die(`unknown arg: ${a} (targets: ${TARGETS.map((x) => x.tag).join(" | ")} | all)`);
    chosen = [t];
  }
  return { targets: (chosen ?? [HOST_TARGET]) as Target[], profile };
}

async function updateDeps() {
  console.error("updating deps to latest");
  if (!(await which("cargo-upgrade"))) await tryRun(["cargo", "install", "cargo-edit"]);
  await tryRun(["cargo", "upgrade", "--incompatible", "--exclude", "cef"]);
  await tryRun(["cargo", "update"]);
  await tryRun(["cargo", "update", "-p", "cef", "--precise", CEF_CRATE_VERSION]); // re-pin cef
  await tryRun(["bun", "update"]);
}

async function tryRun(args: string[]) {
  try {
    await run(args);
  } catch (e) {
    console.error(`skip (non-fatal): ${args.join(" ")} → ${e}`);
  }
}

async function buildMacos(t: Target, profile: string): Promise<string> {
  await cargoBuild(t.triple, "cargo", profile, []);
  await cargoBuild(t.triple, "cargo", profile, ["--lib", "--features", "napi-binding"]);
  const cefFw = await cefFramework(t.triple);

  const distDir = join("dist", t.tag);
  await rm(distDir, { recursive: true, force: true });
  await mkdir(distDir, { recursive: true });
  await run(["bundle-cef-app", "-o", distDir, APP], {
    CEF_PATH: dirname(cefFw),
    CARGO_BUILD_TARGET: t.triple,
  });

  const app = join(distDir, `${APP}.app`);
  for (const h of PRUNED_HELPERS) await rm(join(app, "Contents/Frameworks", h), { recursive: true, force: true });

  const binDir = `target/${t.triple}/${profile}`;
  await cp(join(binDir, "havi"), join(app, "Contents/MacOS/havi"));
  for (const h of ["havi Helper", "havi Helper (GPU)", "havi Helper (Renderer)"]) {
    const dest = join(app, `Contents/Frameworks/${h}.app/Contents/MacOS/${h}`);
    await mkdir(dirname(dest), { recursive: true });
    await cp(join(binDir, "havi-helper"), dest);
  }
  const fwRel = "Contents/Frameworks/Chromium Embedded Framework.framework";
  for (const rel of [
    "Chromium Embedded Framework",
    "Libraries/libEGL.dylib",
    "Libraries/libGLESv2.dylib",
    "Libraries/libcef_sandbox.dylib",
    "Libraries/libvk_swiftshader.dylib",
  ]) {
    const dest = join(app, fwRel, rel);
    await mkdir(dirname(dest), { recursive: true });
    await cp(join(cefFw, rel), dest);
    await stripRelease(dest, profile);
  }

  const ffmpegDest = join(app, "Contents/MacOS/ffmpeg");
  await extractFfmpeg("macos", "arm64", ffmpegDest);
  await chmod(ffmpegDest, 0o755);

  await cp(cdylibPath(t.triple, profile), join(app, "Contents/MacOS/havi.node"));

  await setLsuielement(join(app, "Contents/Info.plist"));
  await codesignBundle(app);
  return app;
}

async function buildUnbundled(t: Target, profile: string): Promise<string> {
  const builder = t.plat === "win32" ? "xwin" : "zigbuild";
  await cargoBuild(t.triple, builder, profile, []);
  await cargoBuild(t.triple, builder, profile, ["--lib", "--features", "napi-binding"]);

  const distDir = join("dist", t.tag);
  await rm(distDir, { recursive: true, force: true });
  await mkdir(distDir, { recursive: true });

  const exe = t.plat === "win32" ? `${APP}.exe` : APP;
  await cp(`target/${t.triple}/${profile}/${exe}`, join(distDir, exe));
  await cp(cdylibPath(t.triple, profile), join(distDir, `${APP}.node`));

  await copyCefRuntime(await cefDir(t.triple), distDir, profile);

  const ffmpegName = t.plat === "win32" ? "ffmpeg.exe" : "ffmpeg";
  const ffmpegDest = join(distDir, ffmpegName);
  await extractFfmpeg(FFMPEG_PLAT[t.plat]!, t.arch, ffmpegDest);
  await chmod(ffmpegDest, 0o755);

  return join(distDir, exe);
}

function cdylibPath(triple: string, profile: string): string {
  const dir = `target/${triple}/${profile}`;
  if (triple.includes("windows")) return `${dir}/${LIB}.dll`;
  if (triple.includes("apple")) return `${dir}/lib${LIB}.dylib`;
  return `${dir}/lib${LIB}.so`;
}

async function cargoBuild(triple: string, builder: string, profile: string, extra: string[]) {
  for (let attempt = 1; attempt <= 3; attempt++) {
    if (builder === "xwin") await patchCefForClangCl(triple);
    const args = ["cargo"];
    if (builder === "zigbuild") args.push("zigbuild", "--target", triple);
    else if (builder === "xwin") args.push("xwin", "build", "--target", triple);
    else args.push("build", "--target", triple);
    if (profile === "release") args.push("--release");
    args.push(...extra);
    try {
      await run(args, { CEF_PATH: cefCache() });
      return;
    } catch (e) {
      console.error(`attempt ${attempt}/3 failed for ${triple}: ${e}`);
      if (builder === "xwin") {
        await rm(`target/${triple}/release/build`, { recursive: true, force: true });
        await rm(`target/${triple}/debug/build`, { recursive: true, force: true });
      }
      if (attempt === 3) throw e;
    }
  }
}

function cefCache(): string {
  return join(cacheDir(), "cef");
}

// Two CEF cmake bugs against clang-cl: /MP (clang-cl errors under /WX) and
// set(CMAKE_CXX_FLAGS "") clears xwin toolchain flags.
async function patchCefForClangCl(triple: string) {
  const dir = await cefDirOrNull(triple);
  if (!dir) return;
  const file = join(dir, "cmake/cef_variables.cmake");
  if (existsSync(file)) {
    const orig = await readFile(file, "utf8");
    let patched = orig.split("/MP\n").join("\n").split("/MP ").join(" ").split(" /MP").join("");
    for (const line of [
      'set(CMAKE_CXX_FLAGS "")',
      'set(CMAKE_CXX_FLAGS_DEBUG "")',
      'set(CMAKE_CXX_FLAGS_RELEASE "")',
    ]) {
      const commented = `# ${line}`;
      if (!patched.includes(commented)) patched = patched.split(line).join(commented);
    }
    if (patched !== orig) await writeFile(file, patched);
  }
  await patchCefDllSysDropDelayimp();
}

// Windows 11 SDK arm64 delayimp.lib is ARM64EC; llvm-lib chokes. Drop from
// CMAKE_STATIC_LINKER_FLAGS — Rust handles dynamic linking anyway.
async function patchCefDllSysDropDelayimp() {
  const registry = join(homedir(), ".cargo/registry/src");
  const tag = "cef-dll-sys-148.1.0+147.0.14";
  const found = await findDir(registry, tag, 3);
  if (!found) return;
  const buildRs = join(found, "build.rs");
  if (!existsSync(buildRs)) return;
  const orig = await readFile(buildRs, "utf8");
  const patched = orig.split('"delayimp.lib",\n').join("");
  if (patched !== orig) await writeFile(buildRs, patched);
}

async function cefDir(triple: string): Promise<string> {
  const d = await cefDirOrNull(triple);
  if (!d) die(`CEF dir missing for ${triple} (expected subdir ${osArchTag(triple)})`);
  return d!;
}

async function cefDirOrNull(triple: string): Promise<string | null> {
  const tag = osArchTag(triple);
  for (const root of [cefCache(), `target/${triple}/debug/build`]) {
    const p = await findDir(root, tag, 6);
    if (p) return p;
  }
  return null;
}

async function cefFramework(triple: string): Promise<string> {
  return join(await cefDir(triple), "Chromium Embedded Framework.framework");
}

function osArchTag(triple: string): string {
  switch (triple) {
    case "aarch64-apple-darwin":
      return "cef_macos_aarch64";
    case "aarch64-unknown-linux-gnu":
      return "cef_linux_aarch64";
    case "x86_64-unknown-linux-gnu":
      return "cef_linux_x86_64";
    case "x86_64-pc-windows-msvc":
      return "cef_windows_x86_64";
    default:
      throw new Error(`unsupported triple: ${triple}`);
  }
}

async function copyCefRuntime(src: string, dst: string, profile: string) {
  const runtimeExts = new Set(["so", "dll", "dylib", "pak", "dat", "bin", "json"]);
  const runtimeFiles = new Set(["chrome-sandbox", "chrome_elf"]);
  const skipDirs = new Set(["cmake", "include", "libcef_dll"]);
  for (const entry of await readdir(src, { withFileTypes: true })) {
    const path = join(src, entry.name);
    if (entry.isFile()) {
      const ext = extname(entry.name).slice(1);
      const keep = runtimeExts.has(ext) || runtimeFiles.has(entry.name);
      if (!keep || ext === "lib") continue;
      const out = join(dst, entry.name);
      await cp(path, out);
      await stripRelease(out, profile);
    } else if (entry.isDirectory() && !skipDirs.has(entry.name)) {
      await cp(path, join(dst, entry.name), { recursive: true });
    }
  }
}

async function stripRelease(path: string, profile: string) {
  if (profile !== "release") return;
  const ext = extname(path);
  const name = basename(path);
  const macho = ext === ".dylib" || name === "Chromium Embedded Framework";
  const elfOrPe = ext === ".so" || ext === ".dll";
  if (!macho && !elfOrPe) return;
  const flag = macho ? "-x" : "--strip-unneeded";
  await run([llvmStripBin(), flag, path]);
}

function llvmStripBin(): string {
  const brew = "/opt/homebrew/opt/llvm/bin/llvm-strip";
  if (existsSync(brew)) return brew;
  return "llvm-strip";
}

async function prefetchFfmpegs(targets: Target[]) {
  const pairs = targets.map((t) => ({ plat: FFMPEG_PLAT[t.plat]!, arch: t.arch }));
  await Promise.all(
    pairs.map(async ({ plat, arch }) => {
      const zip = `ffmpeg-${FFMPEG_VER}-${plat}-${arch}-jellyfin.zip`;
      const cache = join(cacheDir(), zip);
      if (existsSync(cache)) return;
      const url = `https://github.com/seydx/node-av/releases/download/${NODE_AV_TAG}/${zip}`;
      await download(url, cache);
    }),
  );
}

async function extractFfmpeg(plat: string, arch: string, dest: string) {
  const zip = `ffmpeg-${FFMPEG_VER}-${plat}-${arch}-jellyfin.zip`;
  const cache = join(cacheDir(), zip);
  if (!existsSync(cache)) {
    const url = `https://github.com/seydx/node-av/releases/download/${NODE_AV_TAG}/${zip}`;
    await download(url, cache);
  }
  await mkdir(dirname(dest), { recursive: true });
  const buf = await readFile(cache);
  const files = unzipSync(new Uint8Array(buf));
  const first = Object.entries(files).find(([n]) => !n.endsWith("/"));
  if (!first) die(`zip ${cache} empty`);
  await writeFile(dest, first![1]);
}

async function codesignBundle(app: string) {
  const fw = join(app, "Contents/Frameworks/Chromium Embedded Framework.framework");
  await codesign(fw, false);
  for (const e of await readdir(join(app, "Contents/Frameworks"), { withFileTypes: true })) {
    if (e.name.endsWith(".app")) await codesign(join(app, "Contents/Frameworks", e.name), true);
  }
  await codesign(app, true);
  await run(["codesign", "--verify", "--strict", app]);
}

async function codesign(path: string, deep: boolean) {
  const args = ["codesign", "--force", "--sign", IDENT, "--timestamp=none"];
  if (deep) args.push("--deep");
  args.push(path);
  await run(args);
}

async function setLsuielement(plist: string) {
  const pb = "/usr/libexec/PlistBuddy";
  await $`${pb} -c "Add :LSUIElement bool true" ${plist}`.quiet().nothrow();
  await run([pb, "-c", "Set :LSUIElement true", plist]);
}

async function ensureTools() {
  if (!(await which("zig"))) die("zig not on PATH — `brew install zig`");
  if (!(await which("llvm-strip")) && !existsSync("/opt/homebrew/opt/llvm/bin/llvm-strip")) {
    die("llvm-strip missing — `brew install llvm`");
  }
  for (const t of ["cargo-zigbuild", "cargo-xwin"]) {
    if (!(await which(t))) await run(["cargo", "install", t]);
  }
  if (!(await which("bundle-cef-app"))) {
    await run([
      "cargo",
      "install",
      "cef",
      "--version",
      CEF_CRATE_VERSION,
      "--features",
      "build-util",
      "--bin",
      "bundle-cef-app",
    ]);
  }
  if (!(await hasCodesignIdentity())) await createSelfSignedCert();
}

async function hasCodesignIdentity(): Promise<boolean> {
  const out = await $`security find-identity -v -p codesigning`.text();
  return out.includes(`"${IDENT}"`);
}

// Ad-hoc CDHash rotates each rebuild → Gatekeeper rejects helpers with -67030.
async function createSelfSignedCert() {
  const home = homedir();
  const keychain = `${home}/Library/Keychains/login.keychain-db`;
  const dir = (await $`mktemp -d`.text()).trim();
  const conf = join(dir, "cert.conf"),
    key = join(dir, "cert.key"),
    crt = join(dir, "cert.crt"),
    p12 = join(dir, "cert.p12");
  await writeFile(
    conf,
    `[ req ]\ndistinguished_name = dn\nprompt = no\nx509_extensions = v3\n` +
      `[ dn ]\nCN = ${IDENT}\n` +
      `[ v3 ]\nbasicConstraints = critical,CA:FALSE\nkeyUsage = critical,digitalSignature\n` +
      `extendedKeyUsage = critical,codeSigning\n# Apple Code Signing OID — required since macOS 13.\n` +
      `1.2.840.113635.100.6.1.13 = critical,DER:0500\n`,
  );
  await run([
    "openssl",
    "req",
    "-new",
    "-newkey",
    "rsa:2048",
    "-x509",
    "-days",
    "3650",
    "-nodes",
    "-keyout",
    key,
    "-out",
    crt,
    "-config",
    conf,
  ]);
  // macOS Security can't read OpenSSL 3 PBES2 PKCS12 — pin legacy 3DES.
  await run([
    "openssl",
    "pkcs12",
    "-export",
    "-legacy",
    "-keypbe",
    "PBE-SHA1-3DES",
    "-certpbe",
    "PBE-SHA1-3DES",
    "-macalg",
    "sha1",
    "-iter",
    "2048",
    "-out",
    p12,
    "-inkey",
    key,
    "-in",
    crt,
    "-name",
    IDENT,
    "-passout",
    "pass:havi",
  ]);
  await run([
    "security",
    "import",
    p12,
    "-k",
    keychain,
    "-P",
    "havi",
    "-T",
    "/usr/bin/codesign",
    "-T",
    "/usr/bin/security",
  ]);
  await run(["security", "add-trusted-cert", "-p", "codeSign", "-k", keychain, crt]);
  if (!(await hasCodesignIdentity())) die(`identity '${IDENT}' missing after import`);
}

function cacheDir(): string {
  return join(homedir(), ".cache", "havi-build");
}

async function download(url: string, dest: string) {
  await mkdir(dirname(dest), { recursive: true });
  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      console.error(`downloading ${url}${attempt > 1 ? ` (attempt ${attempt}/3)` : ""}`);
      const res = await fetch(url, { redirect: "follow" });
      if (!res.ok) throw new Error(`GET ${url}: ${res.status}`);
      await writeFile(dest, new Uint8Array(await res.arrayBuffer()));
      return;
    } catch (e) {
      if (attempt === 3) throw e;
    }
  }
}

async function which(name: string): Promise<boolean> {
  return (await $`command -v ${name}`.quiet().nothrow()).exitCode === 0;
}

async function run(args: string[], env: Record<string, string> = {}) {
  const proc = Bun.spawn(args, { env: { ...process.env, ...env }, stdio: ["inherit", "inherit", "inherit"] });
  const code = await proc.exited;
  if (code !== 0) throw new Error(`command failed: ${args.join(" ")} → exit ${code}`);
}

async function findDir(root: string, name: string, maxDepth: number): Promise<string | null> {
  if (!existsSync(root)) return null;
  const stack: Array<[string, number]> = [[root, 0]];
  while (stack.length) {
    const [dir, depth] = stack.pop()!;
    if (depth > maxDepth) continue;
    let entries: Dirent[];
    try {
      entries = await readdir(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const e of entries) {
      if (!e.isDirectory()) continue;
      const path = join(dir, e.name);
      if (e.name === name) return path;
      stack.push([path, depth + 1]);
    }
  }
  return null;
}

function die(msg: string): never {
  console.error(msg);
  process.exit(1);
}
