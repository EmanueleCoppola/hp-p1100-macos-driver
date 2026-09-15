#!/bin/sh
# Verify that everything we build and install is native ARM64, with no Rosetta in the path.
#
# The plan's Phase 10 asks for exactly this:
#   "file <our binaries> must show ARM64 or Universal with an ARM64 slice. Verify that
#    running the printing stack does not launch any x86_64 helper process. There should be
#    no dependency on Rosetta."
#
# Apple's otool/lipo are gated behind the Xcode licence on some machines, but file(1) is
# not, and it is enough to tell a native binary from an Intel-only one.
#
# Usage:
#   scripts/verify-native.sh                 check our built and installed binaries
#   scripts/verify-native.sh --include-hp    also report HP's filter, as a control
#   scripts/verify-native.sh --processes     flag any x86_64 process in the print path
#
# Exit status 0 when everything we found is native, 1 otherwise.

set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
INCLUDE_HP=0
MODE=files
FILES=

case "${1:-}" in
    --include-hp) INCLUDE_HP=1 ;;
    --processes) MODE=processes ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    "") ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *) FILES="$*" ;;
esac

# Which architectures does this file contain, and is arm64 among them?
classify() {
    # file(1) prints one extra line per slice for a universal binary; the first line is the
    # summary we want.
    desc=$(/usr/bin/file -b "$1" 2>/dev/null | head -1 || echo 'not a file')
    case "$desc" in
        *arm64*) echo "native   $desc" ;;
        *x86_64*|*i386*) echo "INTEL    $desc" ;;
        *) echo "unknown  $desc" ;;
    esac
}

if [ "$MODE" = processes ]; then
    echo "Processes in the print path:"
    found=0
    for pat in cupsd cupsfilter p1100; do
        for pid in $(pgrep -f "$pat" 2>/dev/null || true); do
            arch=$(/usr/bin/arch -pid "$pid" 2>/dev/null || true)
            [ -n "$arch" ] || continue
            printf '  %6s  %-12s %s\n' "$pid" "$arch" "$(ps -o comm= -p "$pid" 2>/dev/null)"
            case "$arch" in
                x86_64) found=1 ;;
            esac
        done
    done
    if [ "$found" = 0 ]; then
        echo "  no x86_64 process found in the print path"
        echo "  (run this while a job is printing for it to mean anything)"
    fi
    exit "$found"
fi

status=0
depstatus=0
checked=0

# The dependency check needs a working Python; the architecture check does not. Apple's
# /usr/bin/python3 is behind the licence gate, so look for one that runs and say so if there
# is none rather than failing the script.
PYTHON=${PYTHON:-}
if [ -z "$PYTHON" ]; then
    for candidate in /opt/homebrew/bin/python3.13 /opt/homebrew/bin/python3.10 python3; do
        if "$candidate" -c 'pass' >/dev/null 2>&1; then PYTHON=$candidate; break; fi
    done
fi

# What a binary links. The whole point of this replacement is that the answer is libSystem and
# nothing else - no libcups, no frameworks, no rpath - so a stray dependency is a finding, not a
# detail.
# `judge` is 0 for the HP control, whose dependencies are its own business and must not change
# this script's verdict.
deps() {
    judge=${2:-1}
    if [ -z "$PYTHON" ]; then
        echo "      links       not checked: no working Python for the Mach-O reader"
        return 0
    fi
    dylibs=$("$PYTHON" "$REPO/scripts/macho.py" libs "$1" 2>/dev/null \
        | sed -n 's/^ *load.*  \(\/.*\)$/\1/p')
    if [ -z "$dylibs" ]; then
        echo "      links       not a Mach-O file, or unreadable"
        return 0
    fi
    printf '      links       %s\n' "$(echo "$dylibs" | tr '\n' ' ' | sed 's/ $//')"
    for dylib in $dylibs; do
        case "$dylib" in
            /usr/lib/libSystem.B.dylib) ;;
            *) if [ "$judge" = 1 ]; then
                   echo "      UNEXPECTED  $dylib"; depstatus=1
               else
                   echo "      beyond libSystem: $dylib"
               fi ;;
        esac
    done
}

check() {
    verdict=$(classify "$1")
    checked=$((checked + 1))
    printf '  %-22s %s\n' "$(basename "$1")" "$verdict"
    deps "$1"
    case "$verdict" in
        INTEL*) status=1 ;;
    esac
}

if [ -n "$FILES" ]; then
    echo "Requested:"
    for f in $FILES; do check "$f"; done
else
    echo "Our build artifacts:"
    for b in "$REPO"/target/release/p1100 "$REPO"/target/release/p1100-filter; do
        if [ -x "$b" ]; then check "$b"; fi
    done

    echo "Installed:"
    if [ -x /Library/Printers/p1100/p1100-filter ]; then
        check /Library/Printers/p1100/p1100-filter
    else
        echo "  nothing installed at /Library/Printers/p1100"
    fi
fi

if [ "$checked" = 0 ]; then
    echo
    echo "Nothing to check yet: build with './scripts/build-native.sh' and install with"
    echo "./scripts/install.sh, then run this again."
fi

if [ "$INCLUDE_HP" = 1 ]; then
    echo "HP's driver, as a control (expected Intel-only; does not affect the result):"
    HP=/Library/Printers/hp/laserjet/p1100_1560_1600series/hpRaster.bundle/Contents/MacOS
    for b in "$HP/rastertozjs" "$HP/usbink"; do
        if [ -f "$b" ]; then
            printf '  %-22s %s\n' "$(basename "$b")" "$(classify "$b")"
            deps "$b" 0
        fi
    done
fi

echo
if [ "$status" = 0 ] && [ "$depstatus" = 0 ]; then
    echo "OK: nothing Intel-only, and nothing linked beyond libSystem."
elif [ "$status" != 0 ]; then
    echo "FAIL: an Intel-only binary is in the print path." >&2
else
    echo "FAIL: something links more than libSystem (above)." >&2
fi
[ "$status" = 0 ] && [ "$depstatus" = 0 ] && exit 0
exit 1
