#!/bin/sh
# Remove everything install.sh added. Nothing Apple or HP installed is touched.

set -eu

QUEUE=HP_LaserJet_P1102_native
DEST=/Library/Printers/p1005
DRY=0

usage() {
    cat <<EOF
usage: $0 [--dry-run] [--queue NAME]

  --dry-run      print the commands instead of running them
  --queue NAME   queue to delete (default $QUEUE)
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY=1 ;;
        --queue) QUEUE=$2; shift ;;
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

case "$DEST" in
    */hp|*/hp/*|/Library/Printers/PPDs*|/usr/libexec/cups*|/etc/cups*)
        echo "error: refusing to delete $DEST" >&2; exit 1 ;;
esac

if [ "$DRY" = 0 ] && [ "$(id -u)" -ne 0 ]; then
    echo "error: removing $DEST needs root; re-run with sudo" >&2
    exit 1
fi

if lpstat -p "$QUEUE" >/dev/null 2>&1; then
    echo "1. deleting queue '$QUEUE'"
    run lpadmin -x "$QUEUE"
else
    echo "1. queue '$QUEUE' does not exist; nothing to delete"
fi

echo "2. removing $DEST"
if [ -d "$DEST" ]; then
    run rm -rf "$DEST"
else
    echo "   $DEST is not present"
fi

echo
echo "done. the HP queue and the HP driver files were left alone."
