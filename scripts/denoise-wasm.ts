/**
 * Builds `native/wasm/denoise` (DeepFilterNet3) into `web/src/call/denoise/denoise.wasm`, which is committed so the
 * server's build stays npm-only. Next to it goes a hash of everything it's built from; `--check` (CI) fails when the
 * sources changed without a rebuild.
 */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const crate = path.join(root, "native/wasm/denoise");
const out = path.join(root, "web/src/call/denoise/denoise.wasm");
const hashFile = `${out}.sources`;
const SOURCES = ["native/crates/denoise", "native/wasm/denoise/Cargo.toml", "native/wasm/denoise/Cargo.lock", "native/wasm/denoise/src", "scripts/denoise-wasm.ts"];

function files(rel: string): string[] {
  const abs = path.join(root, rel);
  if (statSync(abs).isFile()) return [rel];
  return readdirSync(abs).sort().flatMap((name) => files(path.join(rel, name)));
}

function sourcesHash(): string {
  const hash = createHash("sha256");
  for (const file of SOURCES.flatMap(files)) hash.update(`${file}\0`).update(readFileSync(path.join(root, file))).update("\0");
  return hash.digest("hex");
}

if (process.argv.includes("--check")) {
  if (readFileSync(hashFile, "utf8").trim() !== sourcesHash()) {
    console.error("denoise.wasm is stale: run `npm run denoise:wasm` and commit the result");
    process.exit(1);
  }
  process.exit(0);
}

const rustflags = [
  // tract's ONNX ops pull in rand, whose getrandom has no randomness here without JS glue; DeepFilterNet has no
  // random ops, so nothing calls it
  "--cfg",
  'getrandom_backend="unsupported"',
  // Every browser with AudioWorklet has WASM SIMD, which tract's kernels use
  "-C",
  "target-feature=+simd128",
  // Panic messages carry source paths; keep the build machine's out of the committed file
  `--remap-path-prefix=${homedir()}=~`,
  `--remap-path-prefix=${root}=.`,
];
execFileSync("cargo", ["build", "--release", "--target", "wasm32-unknown-unknown"], {
  cwd: crate,
  stdio: "inherit",
  env: { ...process.env, CARGO_ENCODED_RUSTFLAGS: rustflags.join("\x1f") },
});
copyFileSync(path.join(crate, "target/wasm32-unknown-unknown/release/denoise_wasm.wasm"), out);
writeFileSync(hashFile, `${sourcesHash()}\n`);
console.log(`${path.relative(root, out)}: ${(statSync(out).size / 1e6).toFixed(1)} MB`);
