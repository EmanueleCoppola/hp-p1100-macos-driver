#!/bin/sh
# Run the pure-logic tests on wasm32-wasip1.
#
# Native aarch64-apple-darwin linking needs Apple's ld, which is behind the Xcode
# licence gate on this machine (see docs/testing.md). wasm links with Rust's own
# bundled rust-lld and needs no SDK, so this is the working test path for now.
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
