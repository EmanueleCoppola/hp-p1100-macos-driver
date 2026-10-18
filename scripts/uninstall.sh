#!/bin/sh
# Remove everything install.sh added. Nothing Apple or HP installed is touched.

set -eu

QUEUE=HP_LaserJet_P1100_native
DEST=/Library/Printers/p1100
DRY=0
MARKER=.p1100-installed

usage() {
    cat <<EOF
usage: $0 [--dry-run] [--queue NAME] [--dest DIR]

  --dry-run      print the commands instead of running them
  --queue NAME   queue to delete (default $QUEUE)
  --dest DIR     directory to remove (default $DEST)
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY=1 ;;
        --queue) QUEUE=$2; shift ;;
        --dest) DEST=$2; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

[ "$QUEUE" != HP_LaserJet_Professional_P1102 ] || {
    echo "error: refusing to delete the source HP queue '$QUEUE'" >&2; exit 1; }

run() {
    if [ "$DRY" = 1 ]; then
        printf 'would run: %s\n' "$*"
    else
        "$@"
    fi
}

case "$DEST" in
    */hp|*/hp/*|/Library/Printers/PPDs*|/usr/libexec/cups*|/etc/cups*|/private/etc/cups*|/private/Library/Printers/PPDs*)
        echo "error: refusing to delete $DEST" >&2; exit 1 ;;
esac
# The directory is a command-line argument, so the two ways of pointing a recursive delete at
# something that should never be deleted are refused here rather than trusted.
case "$DEST" in
    /|"") echo "error: refusing to delete '$DEST'" >&2; exit 1 ;;
    /*) ;;
    *) echo "error: $DEST is not an absolute path" >&2; exit 1 ;;
esac
[ -d "$(dirname "$DEST")" ] || { echo "error: destination parent is missing" >&2; exit 1; }
case "$(basename "$DEST")" in .|..) echo "error: --dest must name a directory, not . or .." >&2; exit 1 ;; esac
[ ! -L "$DEST" ] || { echo "error: refusing symlink destination: $DEST" >&2; exit 1; }
DEST_PARENT=$(cd -P "$(dirname "$DEST")" && pwd)
DEST="$DEST_PARENT/$(basename "$DEST")"
case "$DEST" in
    */hp|*/hp/*|/Library/Printers/PPDs*|/usr/libexec/cups*|/etc/cups*|/private/etc/cups*|/private/Library/Printers/PPDs*)
        echo "error: refusing to delete $DEST" >&2; exit 1 ;;
esac
[ -d "$DEST" ] && [ ! -L "$DEST" ] || {
    [ ! -e "$DEST" ] || { echo "error: refusing a non-directory or symlink destination: $DEST" >&2; exit 1; }
}

SYSTEM=0
case "$DEST" in
    /Library/Printers/*) SYSTEM=1 ;;
esac
if [ "$DRY" = 0 ] && [ "$SYSTEM" = 1 ] && [ "$(id -u)" -ne 0 ]; then
    echo "error: removing $DEST needs root; re-run with sudo" >&2
    exit 1
fi

[ -e "$DEST" ] || { echo "1. queue '$QUEUE' was not changed"; echo "2. $DEST is not present"; exit 0; }

owned=0
[ -f "$DEST/$MARKER" ] && owned=1
if [ "$owned" = 0 ] && [ -f "$DEST/p1100-filter" ] && [ -f "$DEST/p1100-usbink" ] && [ -f "$DEST/p1100.ppd" ] \
   && grep -Fq "*cupsFilter: \"application/vnd.cups-raster 100 $DEST/p1100-filter\"" "$DEST/p1100.ppd"; then
    owned=1
fi
[ "$owned" = 1 ] || { echo "error: refusing unowned destination $DEST" >&2; exit 1; }

if lpstat -p "$QUEUE" >/dev/null 2>&1; then
    queue_ppd=/etc/cups/ppd/$QUEUE.ppd
    [ -f "$queue_ppd" ] && [ ! -L "$queue_ppd" ] && grep -Fq "*cupsFilter: \"application/vnd.cups-raster 100 $DEST/p1100-filter\"" "$queue_ppd" \
        || { echo "error: refusing to delete unrelated queue '$QUEUE'" >&2; exit 1; }
    echo "1. deleting queue '$QUEUE'"
    run lpadmin -x "$QUEUE"
else
    echo "1. queue '$QUEUE' does not exist; nothing to delete"
fi

echo "2. removing $DEST"
if [ -d "$DEST" ]; then
    for f in "$MARKER" p1100-filter p1100-usbink p1100.ppd p1100.icns p1100-400.tones p1100-600.tones; do
        [ -e "$DEST/$f" ] && run rm -f "$DEST/$f"
    done
    if [ "$DRY" = 1 ]; then
        echo "   would rmdir $DEST if it is empty"
    else
        rmdir "$DEST" 2>/dev/null || echo "   preserved $DEST because it contains unrelated files"
    fi
else
    echo "   $DEST is not present"
fi

echo
echo "done. the HP queue and the HP driver files were left alone."
