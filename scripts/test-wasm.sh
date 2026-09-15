#!/bin/sh
# Run the test suite on wasm32-wasip1.
#
# scripts/test-native.sh is the primary route: the same suites on the real target, using
# either Apple's toolchain or Xcode's clang called directly. This script exists for a
# machine where neither is available - wasm links with Rust's own bundled rust-lld and
# needs no SDK at all. See docs/testing.md.
#
# Cargo runs test binaries with the package dir as cwd, so the runner is passed as an
# absolute path rather than being configured in .cargo/config.toml.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
runner="$root/scripts/wasi-test-runner.mjs"

exec cargo test \
    --target wasm32-wasip1 \
    --config "target.wasm32-wasip1.runner=[\"node\", \"$runner\"]" \
    "$@"
