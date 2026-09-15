#!/bin/sh
# Build the native ARM64 binaries.
#
# Prefers Apple's toolchain: with the Xcode licence accepted this is just
#
#     cargo build --release
#
# Without it, /usr/bin/cc refuses to run, and rustc's own linker (rust-lld) cannot read this
# SDK's stubs as shipped - macOS 27 uses an architecture token, arm64e.x1, that the LLVM in the
# current Rust toolchain does not know. This script therefore builds a link-only copy of the
# SDK stubs with those tokens removed (scripts/make-stub-sdk.py, non-destructive) and links
# with rust-lld against it.
#
# Either way the result is an ordinary native arm64 binary linked against the real runtime
# libSystem; only the link step differed. Accepting the licence remains the supported route.
#
# Usage:
#   scripts/build-native.sh                 release build
#   scripts/build-native.sh --debug         debug build
#   scripts/build-native.sh --refresh-stub  rebuild the stub SDK first
#   scripts/build-native.sh --status        report which route would be used, then stop
#   scripts/build-native.sh -- <cargo args>
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
PROFILE=--release
REFRESH=0
STATUS_ONLY=0
EXTRA=

while [ $# -gt 0 ]; do
    case "$1" in
        --debug) PROFILE= ;;
        --release) PROFILE=--release ;;
        --refresh-stub) REFRESH=1 ;;
        --status) STATUS_ONLY=1 ;;
        --) shift; EXTRA="$*"; break ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

# A working Python is needed for the stub SDK on the fallback route. Apple's /usr/bin/python3
# is behind the same licence gate.
PYTHON=${PYTHON:-}
if [ -z "$PYTHON" ]; then
    for candidate in python3 /opt/homebrew/bin/python3.13 /opt/homebrew/bin/python3.10; do
        if "$candidate" -c 'pass' >/dev/null 2>&1; then PYTHON=$candidate; break; fi
    done
fi

SR=$(/Users/manu/.cargo/bin/rustc --print sysroot 2>/dev/null || rustc --print sysroot)
CARGO=${CARGO:-/Users/manu/.cargo/bin/cargo}
command -v "$CARGO" >/dev/null 2>&1 || CARGO=cargo

# Is Apple's linker usable? The licence check prints a message but still exits 0, so test the
# output rather than the status.
LICENCE_OK=1
if /usr/bin/cc -v 2>&1 | head -5 | grep -q 'Xcode license'; then
    LICENCE_OK=0
fi

if [ "$LICENCE_OK" = 1 ]; then
    echo "route: Apple's toolchain (the Xcode licence is accepted)"
    [ "$STATUS_ONLY" = 1 ] && exit 0
    cd "$REPO"
    exec "$CARGO" build $PROFILE --offline $EXTRA
fi

echo "route: rust-lld with a rewritten stub SDK"
echo "       (Apple's linker is unavailable until 'sudo xcodebuild -license accept')"
[ "$STATUS_ONLY" = 1 ] && exit 0

if [ -z "$PYTHON" ]; then
    echo "error: no working Python found for the stub SDK; set PYTHON=/path/to/python3" >&2
    echo "       or accept the Xcode licence and use 'cargo build --release' instead" >&2
    exit 1
fi

STUB=${P1005_STUB_SDK:-$HOME/.cache/p1005-stub-sdk}
if [ "$REFRESH" = 1 ] || [ ! -d "$STUB/usr/lib" ]; then
    "$PYTHON" "$REPO/scripts/make-stub-sdk.py" --out "$STUB"
fi
[ -d "$STUB/usr/lib" ] || { echo "error: stub SDK was not created at $STUB" >&2; exit 1; }

cd "$REPO"
RUSTFLAGS="-Clinker=$SR/lib/rustlib/aarch64-apple-darwin/bin/rust-lld -Clink-arg=-syslibroot -Clink-arg=$STUB"
export RUSTFLAGS
exec "$CARGO" build $PROFILE --offline $EXTRA

