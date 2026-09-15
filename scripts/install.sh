#!/bin/sh
# Install the native p1005 filter and create a CUPS queue for the HP LaserJet P1102.
#
# Design rules, intentionally:
#   - The existing HP installation is read but never written. The source PPD is only read.
#   - A new queue is created under a new name; the working (or broken) HP queue is left
#     exactly as it was, so this is reversible and side-by-side.
#   - Everything we add lives under /Library/Printers/p1005, one directory to delete.
#   - No SIP change, no system extension, no kernel extension, nothing disabled.
#
# `--dry-run` prints every command without running it and needs no privileges.

set -eu

QUEUE=HP_LaserJet_P1102_native
SOURCE_QUEUE=HP_LaserJet_Professional_P1102
DEST=/Library/Printers/p1005
REPO=$(cd "$(dirname "$0")/.." && pwd)
FILTER="$REPO/target/release/p1005-filter"
PYTHON=${PYTHON:-python3}
DRY=0
FORCE=0
DEVICE=

usage() {
    cat <<EOF
usage: $0 [--dry-run] [--force] [--queue NAME] [--source-queue NAME] [--device URI]

  --dry-run          print the commands instead of running them (no privileges needed)
  --force            replace an existing queue of the same name
  --queue NAME       queue to create            (default $QUEUE)
  --source-queue N   HP queue to inherit the device URI and PPD from (default $SOURCE_QUEUE)
  --device URI       use this device URI instead of reading it from the source queue
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY=1 ;;
        --force) FORCE=1 ;;
        --queue) QUEUE=$2; shift ;;
        --source-queue) SOURCE_QUEUE=$2; shift ;;
        --device) DEVICE=$2; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

run() {
    if [ "$DRY" = 1 ]; then
        printf 'would run: %s\n' "$*"
    else
        "$@"
    fi
}

die() { echo "error: $*" >&2; exit 1; }

# --- safety rails -----------------------------------------------------------

case "$DEST" in
    */hp|*/hp/*|/Library/Printers/PPDs*|/usr/libexec/cups*|/etc/cups*)
        die "refusing to install into $DEST: that is an Apple or HP location" ;;
esac
[ -n "$QUEUE" ] || die "empty queue name"

if [ "$DRY" = 0 ] && [ "$(id -u)" -ne 0 ]; then
    die "writing to $DEST and creating a queue need root; re-run with sudo"
fi

# --- check what we are installing -------------------------------------------

if [ ! -x "$FILTER" ]; then
    if [ "$DRY" = 1 ]; then
        echo "note: $FILTER does not exist yet; a real run would stop here."
        echo "      build it with 'cargo build --release' (and, if that fails with an"
        echo "      Xcode licence error: sudo xcodebuild -license accept)"
    else
        die "$FILTER not found; build it first:
    cargo build --release
  (if that fails with an Xcode licence error, run: sudo xcodebuild -license accept)"
    fi
else
    arch=$(/usr/bin/file -b "$FILTER")
    case "$arch" in
        *arm64*) echo "filter: $FILTER ($arch)" ;;
        *) die "$FILTER is not an ARM64 binary: $arch" ;;
    esac
fi

# --- device URI -------------------------------------------------------------

if [ -z "$DEVICE" ]; then
    DEVICE=$(lpstat -v "$SOURCE_QUEUE" 2>/dev/null | sed -n 's/^device for [^:]*: //p')
    [ -n "$DEVICE" ] || die "cannot read the device URI from queue '$SOURCE_QUEUE'; pass --device URI"
fi
echo "device: $DEVICE"

# --- PPD source -------------------------------------------------------------

SRC_PPD=/etc/cups/ppd/$SOURCE_QUEUE.ppd
if [ ! -r "$SRC_PPD" ]; then
    SRC_PPD=/Library/Printers/PPDs/Contents/Resources/hp1100.ppd.gz
fi
[ -r "$SRC_PPD" ] || die "cannot find a source PPD; expected $SRC_PPD"
echo "source PPD: $SRC_PPD"

PYTHON_OK=1
if ! "$PYTHON" -c 'pass' >/dev/null 2>&1; then
    PYTHON_OK=0
    if [ "$DRY" = 0 ]; then
        die "'$PYTHON' does not run (Apple's /usr/bin/python3 is gated behind the Xcode
  licence). Install Homebrew python and re-run, e.g. PYTHON=/opt/homebrew/bin/python3.13 $0"
    fi
fi
[ "$PYTHON_OK" = 1 ] || echo "note: '$PYTHON' is not runnable here; PPD generation would fail"

# --- act --------------------------------------------------------------------

echo
echo "1. installing the filter and PPD into $DEST"
run mkdir -p "$DEST"
run /usr/bin/install -m 0755 -o root -g wheel "$FILTER" "$DEST/p1005-filter"
run "$PYTHON" "$REPO/scripts/make-ppd.py" "$SRC_PPD" "$DEST/p1005-filter" "$DEST/p1005.ppd"

echo
echo "2. creating the queue '$QUEUE'"
if [ "$FORCE" = 1 ]; then
    run lpadmin -x "$QUEUE" 2>/dev/null || true
fi
run lpadmin -p "$QUEUE" -E -v "$DEVICE" -P "$DEST/p1005.ppd"
run lpadmin -p "$QUEUE" -o printer-is-shared=false
run cupsenable "$QUEUE"
run cupsaccept "$QUEUE"

echo
echo "done. the HP installation under /Library/Printers/hp was not touched."
echo
echo "try it:"
echo "    lp -d $QUEUE /path/to/file.pdf"
echo "    lpstat -o $QUEUE        # job state"
echo "    /usr/bin/log show --predicate 'process == \"cupsd\"' --last 5m   # job log"
echo
echo "remove it again with: sudo $REPO/scripts/uninstall.sh --queue $QUEUE"
