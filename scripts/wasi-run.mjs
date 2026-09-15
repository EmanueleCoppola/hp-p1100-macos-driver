// Run a wasm32-wasip1 build of our own command line tool.
//
// Why this exists: linking a native aarch64-apple-darwin binary needs Apple's linker,
// which is behind the Xcode licence gate on this machine (see docs/testing.md). Linking
// wasm uses Rust's own bundled rust-lld and needs no SDK, so the *real* code can still be
// executed here - which is how the Rust job builder gets compared against the
// reference-verified Python path rather than only being unit tested.
//
// Usage:
//   cargo build --target wasm32-wasip1 -p p1005-cli --offline
//   node scripts/wasi-run.mjs target/wasm32-wasip1/debug/p1005.wasm <args...>

import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';

const [wasmPath, ...args] = process.argv.slice(2);
if (!wasmPath) {
  console.error('usage: wasi-run.mjs <program.wasm> [args...]');
  process.exit(2);
}

const wasi = new WASI({
  version: 'preview1',
  // argv[0] is conventionally the program name
  args: [wasmPath.split('/').pop(), ...args],
  env: process.env,
  // The host root is exposed at the guest root, so the tool's absolute paths resolve to
  // the same files the native build would see.
  preopens: { '/': '/' },
  returnOnExit: true,
});

const module = await WebAssembly.compile(await readFile(wasmPath));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
const code = wasi.start(instance);
if (process.env.P1005_WASI_DEBUG) {
  console.error('[wasi-run] guest returned exit code', code);
}
process.exit(code ?? 0);
