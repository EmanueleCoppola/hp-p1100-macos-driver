#!/bin/sh
# Run the installer's file steps into a scratch directory, and take them away again.
#
# Why: verify-all.sh runs install.sh --dry-run, which prints the commands, and scripts/ppd-compare.py
# judges the PPD it generates - but nothing had ever executed the install itself: the copies, the
# tone tables read out of HP's driver, the PPD written into the destination, the removals. That is
# the part a real install performs as root, and the part that would leave a half-installed
# directory behind if a step failed. This runs all of it except lpadmin, into a directory that is
# deleted at the end, and checks what landed there and what was taken away.
#
# The one thing it cannot reach is the strict cupstestppd run: its two file checks want root-owned
# files, which only the real install provides. Everything else is the same code path install.sh
# takes, with the same arguments it would be given for real.
#
# It also runs the refusals, so the rails around a recursive delete are exercised rather than
# assumed: an HP or Apple location, /, a relative path, and the system destination without root.
#
# Usage:
#   scripts/rehearse-install.sh [--keep]
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
cd "$REPO"

QUEUE=HP_LaserJet_P1102_rehearsal
SYSTEM_DEST=/Library/Printers/p1102
SOURCE_QUEUE=HP_LaserJet_Professional_P1102
KEEP=0

while [ $# -gt 0 ]; do
    case "$1" in
        --keep) KEEP=1 ;;
        -h|--help) sed -n '/^# /s/^# {0,1}//p' "$0" | sed -n '/^Usage:/,$p'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

work=$(mktemp -d -t p1102-install-rehearsal)
DEST="$work/p1102"
failures=0

if [ "$KEEP" = 0 ]; then
    trap 'rm -rf "$work"' EXIT INT TERM
fi

ok() { printf '  %-58s ok\n' "$1"; }
bad() { printf '  %-58s FAILED: %s\n' "$1" "$2"; failures=$((failures + 1)); }

echo 'rehearsing the install, into a scratch directory'

[ -x "$REPO/target/release/p1102-filter" ] || {
    echo 'error: build the filter first: scripts/build-native.sh' >&2; exit 2; }
[ -f "$REPO/assets/p1102.icns" ] || {
    echo 'error: build the icon first: scripts/make-icns.py' >&2; exit 2; }

device_before=$(lpstat -v "$SOURCE_QUEUE" 2>/dev/null || true)

# --- the install, with the two arguments that keep it out of everyone's way -------------
if ! "$REPO/scripts/install.sh" --dest "$DEST" --no-queue --queue "$QUEUE" >"$work/install.log" 2>&1
then
    sed 's/^/    /' "$work/install.log" >&2
    echo 'error: install.sh failed' >&2
    exit 1
fi

# --- what should be there --------------------------------------------------------------
for f in p1102-filter p1102.ppd p1102.icns; do
    [ -f "$DEST/$f" ] && ok "$f is in the destination" || bad "$f is in the destination" 'missing'
done
[ -x "$DEST/p1102-filter" ] && ok 'the installed filter is executable' \
    || bad 'the installed filter is executable' 'not executable'
cmp -s "$DEST/p1102-filter" "$REPO/target/release/p1102-filter" \
    && ok 'the installed filter is the build, byte for byte' \
    || bad 'the installed filter is the build, byte for byte' 'differs'
cmp -s "$DEST/p1102.icns" "$REPO/assets/p1102.icns" \
    && ok 'the installed icon is the built one' \
    || bad 'the installed icon is the built one' 'differs'

# The tone tables are HP's data, read out of HP's driver; on a machine without it the installer
# says so and installs nothing there, which is the documented behaviour rather than a failure.
if [ -f "$DEST/p1102-400.tones" ] && [ -f "$DEST/p1102-600.tones" ]; then
    four=$(wc -c <"$DEST/p1102-400.tones" | tr -d ' ')
    six=$(wc -c <"$DEST/p1102-600.tones" | tr -d ' ')
    [ "$four" = 12288 ] && [ "$six" = 12288 ] \
        && ok 'both tone tables are the 12288-byte tables' \
        || bad 'both tone tables are the 12288-byte tables' "$four and $six bytes"
else
    grep -q 'no HP driver to read the tone tables' "$work/install.log" \
        && ok 'no tone tables, and the installer said why' \
        || bad 'no tone tables, and the installer said why' 'missing without a note'
fi

grep -q "^\*cupsFilter: \"application/vnd.cups-raster 100 $DEST/p1102-filter\"" "$DEST/p1102.ppd" \
    && ok 'the PPD points its raster filter into the destination' \
    || bad 'the PPD points its raster filter into the destination' 'no such line'
grep -q "^\*APPrinterIconPath: \"$DEST/p1102.icns\"$" "$DEST/p1102.ppd" \
    && ok 'the PPD points its icon into the destination' \
    || bad 'the PPD points its icon into the destination' 'no such line'

# --- what should not have happened ------------------------------------------------------
[ ! -e "$SYSTEM_DEST" ] && ok "$SYSTEM_DEST was not created" \
    || bad "$SYSTEM_DEST was not created" 'it exists'
[ "$(lpstat -v "$SOURCE_QUEUE" 2>/dev/null || true)" = "$device_before" ] \
    && ok 'the HP queue still points where it did' \
    || bad 'the HP queue still points where it did' 'the device changed'
if lpstat -p "$QUEUE" >/dev/null 2>&1; then
    bad 'no queue was created' "$QUEUE exists"
else
    ok 'no queue was created'
fi

# --- and the uninstall takes it all away -----------------------------------------------
if ! "$REPO/scripts/uninstall.sh" --dest "$DEST" --queue "$QUEUE" >"$work/uninstall.log" 2>&1
then
    sed 's/^/    /' "$work/uninstall.log" >&2
    echo 'error: uninstall.sh failed' >&2
    exit 1
fi
[ ! -e "$DEST" ] && ok 'the uninstall removed the destination' \
    || bad 'the uninstall removed the destination' 'it is still there'

# --- the refusals ----------------------------------------------------------------------
for bad_dest in /Library/Printers/hp /Library/Printers/PPDs/Contents/Resources /etc/cups / \
                relative-directory ''; do
    if "$REPO/scripts/uninstall.sh" --dry-run --dest "$bad_dest" >/dev/null 2>&1; then
        bad "uninstall refuses --dest '$bad_dest'" 'it did not refuse'
    else
        ok "uninstall refuses --dest '$bad_dest'"
    fi
done

for bad_dest in /Library/Printers/hp /Library/Printers/PPDs/Contents/Resources; do
    if "$REPO/scripts/install.sh" --dry-run --dest "$bad_dest" >/dev/null 2>&1; then
        bad "install refuses --dest '$bad_dest'" 'it did not refuse'
    else
        ok "install refuses --dest '$bad_dest'"
    fi
done

# Non-root, at the system destination: the refusal whose message tells the user what to do.
if "$REPO/scripts/install.sh" --dest "$SYSTEM_DEST" --no-queue >/dev/null 2>&1; then
    bad 'install refuses the system destination without root' 'it did not refuse'
else
    ok 'install refuses the system destination without root'
fi
if "$REPO/scripts/uninstall.sh" --dest "$SYSTEM_DEST" >/dev/null 2>&1; then
    bad 'uninstall refuses the system destination without root' 'it did not refuse'
else
    ok 'uninstall refuses the system destination without root'
fi

echo
if [ "$failures" != 0 ]; then
    echo "$failures check(s) failed in the rehearsal" >&2
    exit 1
fi
echo 'the install and the uninstall both did what they claim, in a scratch directory.'
if [ "$KEEP" = 1 ]; then
    echo "kept: $work"
else
    echo "the scratch directory $work is removed on exit"
fi
exit 0
