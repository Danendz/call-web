/**
 * Brotli copies of the build's big binaries, next to them as `<file>.br`; the server sends one to a browser that
 * takes it. The noise suppression WASM is ~22 MB, and ~10 MB compressed: on a slow link into mainland China that's
 * the difference that matters.
 */
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { brotliCompressSync, constants } from "node:zlib";

const dir = path.resolve(import.meta.dirname, "../dist/web/assets");
for (const name of readdirSync(dir).filter((n) => n.endsWith(".wasm"))) {
  const file = path.join(dir, name);
  const body = readFileSync(file);
  const compressed = brotliCompressSync(body, {
    params: {
      [constants.BROTLI_PARAM_QUALITY]: constants.BROTLI_MAX_QUALITY,
      [constants.BROTLI_PARAM_SIZE_HINT]: body.length,
    },
  });
  writeFileSync(`${file}.br`, compressed);
  console.log(`${name}: ${(body.length / 1e6).toFixed(1)} MB, ${(compressed.length / 1e6).toFixed(1)} MB as brotli`);
}
