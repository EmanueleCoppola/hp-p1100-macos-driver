#!/bin/sh
# The reference driver (foo2zjs) as a *test oracle*.
#
# Why this exists: the P1102 is supported by foo2zjs, so a job built by it is the closest
# thing to a working driver's output that can be produced on this machine - HP's own filter
# is x86_64 and cannot run here at all. scripts/reference-compare.py uses this to check our
# job against it byte for byte.
#
# foo2zjs is GPL-2.0. It is fetched, built and run out of a cache directory and is never
# committed, linked or shipped from this repository, exactly like the jbigkit bottle that
# scripts/jbig-oracle.sh fetches. See docs/research.md.
#
# Usage:
#   scripts/foo2zjs-oracle.sh fetch      download and build it (idempotent)
#   scripts/foo2zjs-oracle.sh which      print the binary path
#   scripts/foo2zjs-oracle.sh <args...>  run it with those arguments
#
# Building needs a C compiler and make. On this machine /usr/bin/cc and /usr/bin/make are
# behind the Xcode licence prompt, while Xcode's own toolchain binaries are not, so this
# script calls those directly with an explicit -isysroot. Accepting the licence remains the
# supported route, and is what the lookups below prefer if it has been accepted.

set -eu

ORACLE_DIR=${P1100_FOO2ZJS_DIR:-$HOME/.cache/p1100-foo2zjs}
SRC="$ORACLE_DIR/src"
URL=https://github.com/OpenPrinting/foo2zjs/archive/refs/heads/master.tar.gz
# Pinned like the jbigkit bottle. The tarball tracks upstream's branch, so if upstream
# moves, the checksum changes and this stops rather than silently testing against
# something else: check the new tarball and set P1100_FOO2ZJS_SHA to accept it.
SHA=${P1100_FOO2ZJS_SHA:-77317fdc1c8282c28f1410d557c521b1d7d7201a208854d2a0910f47b95b839b}

TOOLCHAIN=/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin
CLT_TOOLCHAIN=/Library/Developer/CommandLineTools/usr/bin
SDK_CANDIDATES="/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk
/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk"

# A tool is usable if running it does not hit the licence shim.
works() { "$@" >/dev/null 2>&1; }

find_cc() {
    if works cc --version; then printf 'cc'; return; fi
    for c in "$TOOLCHAIN/clang" "$CLT_TOOLCHAIN/clang"; do
        if works "$c" --version; then printf '%s' "$c"; return; fi
    done
    printf 'error: no working C compiler; accept the Xcode licence or install one\n' >&2
    exit 1
}

find_make() {
    if works make --version; then printf 'make'; return; fi
    for m in /Applications/Xcode.app/Contents/Developer/usr/bin/make \
             "$TOOLCHAIN/make"; do
        if works "$m" --version; then printf '%s' "$m"; return; fi
    done
    printf 'error: no working make; accept the Xcode licence or install one\n' >&2
    exit 1
}

find_sdk() {
    sdk=$(xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)
    if [ -n "$sdk" ] && [ -d "$sdk" ]; then printf '%s' "$sdk"; return; fi
    for s in $SDK_CANDIDATES; do
        if [ -d "$s" ]; then printf '%s' "$s"; return; fi
    done
    printf 'error: no macOS SDK found; install Xcode or the command line tools\n' >&2
    exit 1
}

fetch() {
    if [ -x "$SRC/foo2zjs" ]; then
        echo "oracle already built in $SRC"
        return 0
    fi
    mkdir -p "$ORACLE_DIR"
    tgz="$ORACLE_DIR/foo2zjs.tar.gz"
    if [ ! -f "$tgz" ]; then
        echo "fetching foo2zjs (test oracle only, GPL)"
        curl -fsSL -m 180 -o "$tgz" "$URL"
    fi
    actual=$(shasum -a 256 "$tgz" | cut -d' ' -f1)
    if [ "$actual" != "$SHA" ]; then
        echo "error: tarball checksum mismatch" >&2
        echo "  expected $SHA" >&2
        echo "  actual   $actual" >&2
        echo "  upstream may have moved; inspect the new tarball and set P1100_FOO2ZJS_SHA" >&2
        exit 1
    fi
    mkdir -p "$ORACLE_DIR/extract"
    tar xzf "$tgz" -C "$ORACLE_DIR/extract"
    top=$(find "$ORACLE_DIR/extract" -maxdepth 1 -mindepth 1 -type d | head -1)
    [ -n "$top" ] || { echo 'error: tarball had no top-level directory' >&2; exit 1; }
    mkdir -p "$SRC"
    (cd "$top" && tar cf - .) | (cd "$SRC" && tar xf -)

    cc=$(find_cc)
    mk=$(find_make)
    sdk=$(find_sdk)
    case "$cc" in
        cc) CC="cc -isysroot $sdk" ;;
        *)  CC="$cc -isysroot $sdk" ;;
    esac
    echo "building foo2zjs with $cc and SDK $sdk"
    # foo2zjs links -ljbig normally. The tarball ships its own copy of Markus Kuhn's
    # encoder, which is what we build instead, so no library install is needed.
    (cd "$SRC" && "$mk" CC="$CC" jbig.o jbig_ar.o >/dev/null 2>&1 \
        && "$mk" CC="$CC" LIBJBG="jbig.o jbig_ar.o" foo2zjs zjsdecode)
    [ -x "$SRC/foo2zjs" ] || { echo 'error: build failed' >&2; exit 1; }
    echo "oracle ready: $SRC/foo2zjs"
}

case ${1:-} in
    fetch) fetch ;;
    which) echo "$SRC/foo2zjs" ;;
    "")   sed -n '10,14p' "$0"; exit 2 ;;
    *)    [ -x "$SRC/foo2zjs" ] || { echo "error: run '$0 fetch' first" >&2; exit 1; }
          exec "$SRC/foo2zjs" "$@" ;;
esac
