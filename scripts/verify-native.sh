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
    for pat in cupsd cupsfilter p1102; do
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
checked=0

check() {
    verdict=$(classify "$1")
    checked=$((checked + 1))
    printf '  %-22s %s\n' "$(basename "$1")" "$verdict"
    case "$verdict" in
        INTEL*) status=1 ;;
    esac
}

if [ -n "$FILES" ]; then
    echo "Requested:"
    for f in $FILES; do check "$f"; done
else
    echo "Our build artifacts:"
    for b in "$REPO"/target/release/p1102 "$REPO"/target/release/p1102-filter; do
        if [ -x "$b" ]; then check "$b"; fi
    done

    echo "Installed:"
    if [ -x /Library/Printers/p1102/p1102-filter ]; then
        check /Library/Printers/p1102/p1102-filter
    else
        echo "  nothing installed at /Library/Printers/p1102"
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
        if [ -f "$b" ]; then printf '  %-22s %s\n' "$(basename "$b")" "$(classify "$b")"; fi
    done
fi

echo
if [ "$status" = 0 ]; then
    echo "OK: nothing Intel-only found."
else
    echo "FAIL: an Intel-only binary is in the print path." >&2
fi
exit "$status"
