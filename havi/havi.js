#!/usr/bin/env node
import { spawn } from "node:child_process";
import { arch, platform } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SUPPORTED = new Set(["darwin-arm64", "linux-arm64", "linux-x64", "win32-x64"]);
const target = `${platform()}-${arch()}`;
if (!SUPPORTED.has(target)) throw new Error(`havi: unsupported platform ${target}`);

const dist = join(dirname(fileURLToPath(import.meta.url)), "..", "dist", target);
const bin =
  platform() === "darwin" ? join(dist, "havi.app/Contents/MacOS/havi") :
  platform() === "win32"  ? join(dist, "havi.exe") :
                            join(dist, "havi");

const child = spawn(bin, process.argv.slice(2), { stdio: "inherit" });
child.on("exit", (code, signal) => signal ? process.kill(process.pid, signal) : process.exit(code ?? 0));
