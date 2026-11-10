# Bergamot

Firefox's on-device translation engine ([bergamot-translator](https://github.com/mozilla/bergamot-translator), Marian compiled to WebAssembly), used for live subtitles on both clients:

- `bergamot.wasm`: the engine, from Firefox's Remote Settings collection `translations-wasm`, record `bergamot-translator`.
- `glue.js`: its Emscripten and embind loader, built by the same pipeline (`BERGAMOT_VERSION_FULL` inside says which build). It's unchanged.

The web app loads both in a Worker (`web/src/translate/`). The native app compiles `bergamot.wasm` ahead of time with wasmtime in `native/crates/engine/build.rs` and runs it through its own embind host (`engine/src/translate/`), so the two always run the same engine.

Both files are MPL-2.0. The models (one per direction, ~52 MB each, also MPL-2.0, from Firefox's `translations-models` collection) aren't in the repo. They are hosted in the `releases` bucket, served at `/updates/bergamot-*`, and checked against SHA-256s in `web/src/translate/models.ts` and `engine/src/translate/model.rs`.

To update, replace both files from the same Remote Settings release. Then check that the native host's output still matches, using the `translate` example in the engine.

| File | SHA-256 |
|---|---|
| `bergamot.wasm` | `a3a89d9ad0a4ed8f27bf3e403701b23f5709816f6376438503f2fa5b0182c2dc` |
| `glue.js` | `faff1ef6285b0d26f01787776fd49299dfb756ecb9688aa990c250e66797b47d` |
