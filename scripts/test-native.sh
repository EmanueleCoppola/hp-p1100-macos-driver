#!/bin/sh
# Run the test suite on this machine's own architecture.
#
# Prefers the plain route:
#
#     cargo test --workspace
#
# which needs Apple's linker. When the Xcode licence has not been accepted, /usr/bin/cc and
# the shims around clang refuse - but Xcode's own clang does not, so this script passes it
# to rustc as the linker together with an explicit SDK. Nothing is accepted or installed;
# accepting the licence remains the supported route. See docs/testing.md.
#
# scripts/test-wasm.sh runs the same suite on wasm32-wasip1 with Rust's bundled linker, for
# a machine with no usable native toolchain at all. The suites are the same; native is the
# real target, so it is what this script runs.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

if cc -v >/dev/null 2>&1; then
    exec cargo test --offline --workspace "$@"
fi

TOOLCHAIN=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin
CLT_TOOLCHAIN=/Library/Developer/CommandLineTools/usr/bin

find_sdk() {
    sdk=$(xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)
    if [ -n "$sdk" ] && [ -d "$sdk" ]; then printf '%s' "$sdk"; return 0; fi
    for s in \
        /Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk \
        /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk; do
        if [ -d "$s" ]; then printf '%s' "$s"; return 0; fi
    done
    return 1
}

for c in "$TOOLCHAIN/clang" "$CLT_TOOLCHAIN/clang"; do
    if "$c" --version >/dev/null 2>&1; then
        sdk=$(find_sdk) || { echo "error: no macOS SDK found" >&2; exit 1; }
        echo "linking with $c (the /usr/bin shims are gated, this is not)" >&2
        RUSTFLAGS="-Clinker=$c -Clink-arg=-isysroot -Clink-arg=$sdk"
        SDKROOT=$sdk
        export RUSTFLAGS SDKROOT
        exec cargo test --offline --workspace "$@"
    fi
done

echo "error: no usable native toolchain; use scripts/test-wasm.sh instead" >&2
exit 1
