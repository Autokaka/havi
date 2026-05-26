import { createRequire } from "node:module";
import { arch, platform } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SUPPORTED = new Set(["darwin-arm64", "linux-arm64", "linux-x64", "win32-x64"]);
const target = `${platform()}-${arch()}`;
if (!SUPPORTED.has(target)) throw new Error(`havi: unsupported platform ${target}`);

const dist = join(dirname(fileURLToPath(import.meta.url)), "..", "dist", target);
const nodePath = platform() === "darwin"
  ? join(dist, "havi.app/Contents/MacOS/havi.node")
  : join(dist, "havi.node");

export const havi = createRequire(import.meta.url)(nodePath);
