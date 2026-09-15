// Cargo test runner for wasm32-wasip1.
//
// Why: linking native aarch64-apple-darwin binaries needs Apple's linker, which is
// gated behind the Xcode licence on this machine (see docs/testing.md). Linking wasm
// uses Rust's own bundled rust-lld and needs no SDK, so pure-logic tests can still be
// executed. Used via `.cargo/config.toml`:
//
//     cargo test --target wasm32-wasip1
//
// Invoked by cargo as: node scripts/wasi-test-runner.mjs <test-binary.wasm> [args...]

import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';

const [wasmPath, ...rest] = process.argv.slice(2);
if (!wasmPath) {
  console.error('usage: wasi-test-runner.mjs <test-binary.wasm> [args...]');
  process.exit(2);
}

// libtest spawns threads by default; wasm has none, so force serial execution.
let args = rest.includes('--test-threads') || rest.some((a) => a.startsWith('--test-threads='))
  ? rest
  : [...rest, '--test-threads=1'];

// wasm has no unwinding, so libtest cannot catch a panic and never regains control to
// print a test's captured output. With --nocapture the panic message goes straight to
// the real stderr and is actually visible, which is the difference between a useful
// failure and an opaque `RuntimeError: unreachable`.
if (!args.includes('--nocapture')) {
  args = [...args, '--nocapture'];
}

const wasi = new WASI({
  version: 'preview1',
  args: [wasmPath, ...args],
  env: {},
  preopens: {},
  returnOnExit: true,
});

const module = await WebAssembly.compile(await readFile(wasmPath));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
const code = wasi.start(instance);
process.exit(code ?? 0);
