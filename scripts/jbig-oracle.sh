#!/bin/sh
# Reference JBIG (T.82) oracle, for testing our encoder against a known-good one.
#
# Why this exists: our encoder has to produce output a real decoder accepts, and there is
# no compiler available here to build a reference implementation. Homebrew ships jbigkit
# as a *prebuilt* bottle, so unpacking it needs no toolchain at all.
#
# jbigkit is GPL. It is used strictly as a test oracle: never committed, never linked,
# never shipped. See docs/jbig.md.
#
# Usage:
#   scripts/jbig-oracle.sh fetch                      download and verify the bottle
#   scripts/jbig-oracle.sh encode <in.pbm> <out.jbg> [extra pbmtojbg args...]
#   scripts/jbig-oracle.sh decode <in.jbg> <out.pbm>
#   scripts/jbig-oracle.sh verify <ours.jbg> <expected.pbm>
#   scripts/jbig-oracle.sh which                      print the resolved oracle directory
#
# 'encode' uses the parameters we target: a single resolution layer (-d 0), options = 0
# (no typical prediction, no differential layers, no LRLTWO), MX = 0 (no adaptive template
# movement), L0 = 128. Our encoder must reproduce its output byte for byte.
#
# -d 0 matters: pbmtojbg otherwise silently enables differential layers for images larger
# than its default 640x480 lowest layer, which produces a different BIH and a stream our
# (single-layer) encoder should not be compared against.

set -eu

ORACLE_DIR=${P1005_ORACLE_DIR:-$HOME/.cache/p1005-jbig-oracle}
BOTTLE_URL='https://ghcr.io/v2/homebrew/core/jbigkit/blobs/sha256:a7cc930c8a3652caa237eb15f5528d6a9384ba2e14b7414de8946fe0fedf6816'
BOTTLE_SHA='a7cc930c8a3652caa237eb15f5528d6a9384ba2e14b7414de8946fe0fedf6816'
VERSION=2.1

# Apple's /usr/bin/python3 refuses to run until the Xcode licence is accepted, so prefer
# whatever actually works.
PYTHON=${PYTHON:-python3}
for candidate in "$PYTHON" /opt/homebrew/bin/python3.13 /opt/homebrew/bin/python3.10; do
    if "$candidate" -c 'pass' >/dev/null 2>&1; then PYTHON=$candidate; break; fi
done

HERE=$(cd "$(dirname "$0")" && pwd)

oracle_bin() {
    printf '%s/jbigkit/%s/bin/%s' "$ORACLE_DIR" "$VERSION" "$1"
}

with_libs() {
    DYLD_FALLBACK_LIBRARY_PATH="$ORACLE_DIR/jbigkit/$VERSION/lib" "$@"
}

need() {
    [ -x "$(oracle_bin pbmtojbg)" ] || { echo "error: run '$0 fetch' first" >&2; exit 1; }
}

fetch() {
    if [ -x "$(oracle_bin pbmtojbg)" ]; then
        echo "oracle already present in $ORACLE_DIR"
        return 0
    fi
    echo "fetching jbigkit $VERSION bottle (test oracle only, GPL)"
    mkdir -p "$ORACLE_DIR"
    tgz="$ORACLE_DIR/jbigkit-bottle.tar.gz"
    token=$(curl -sS -m 30 'https://ghcr.io/token?service=ghcr.io&scope=repository:homebrew/core/jbigkit:pull' \
        | "$PYTHON" -c 'import json,sys; print(json.load(sys.stdin)["token"])')
    curl -sS -m 180 -L -H "Authorization: Bearer $token" "$BOTTLE_URL" -o "$tgz"
    actual=$(shasum -a 256 "$tgz" | cut -d' ' -f1)
    if [ "$actual" != "$BOTTLE_SHA" ]; then
        echo "error: bottle checksum mismatch" >&2
        echo "  expected $BOTTLE_SHA" >&2
        echo "  actual   $actual" >&2
        exit 1
    fi
    tar -xzf "$tgz" -C "$ORACLE_DIR"
    rm -f "$tgz"
    need
    echo "oracle ready in $ORACLE_DIR"
}

encode() {
    need
    [ $# -ge 2 ] || { echo "usage: $0 encode <in.pbm> <out.jbg> [extra args...]" >&2; exit 2; }
    in=$1; out=$2; shift 2
    with_libs "$(oracle_bin pbmtojbg)" -d 0 -p 0 -m 0 -s 128 "$@" "$in" "$out"
}

decode() {
    need
    [ $# -eq 2 ] || { echo "usage: $0 decode <in.jbg> <out.pbm>" >&2; exit 2; }
    with_libs "$(oracle_bin jbgtopbm)" "$1" "$2"
}

verify() {
    need
    [ $# -eq 2 ] || { echo "usage: $0 verify <ours.jbg> <expected.pbm>" >&2; exit 2; }
    tmp=$(mktemp -t p1005-verify)
    decode "$1" "$tmp"
    "$PYTHON" "$HERE/pbm-compare.py" "$2" "$tmp"
    rm -f "$tmp"
}

case ${1:-} in
    fetch) fetch ;;
    encode) shift; encode "$@" ;;
    decode) shift; decode "$@" ;;
    verify) shift; verify "$@" ;;
    which) echo "$ORACLE_DIR" ;;
    *) sed -n '10,19p' "$0"; exit 2 ;;
esac
