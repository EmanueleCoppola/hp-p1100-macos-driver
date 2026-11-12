#!/bin/sh
# Install the printer utility and update an existing p1100 CUPS queue in place.
set -eu
REPO=$(cd "$(dirname "$0")/.." && pwd)
QUEUE=${1:-HP_LaserJet_P1100_native}
DEST=/Library/Printers/p1100
APP="$REPO/target/P1100 Utility.app"
PPD="/etc/cups/ppd/$QUEUE.ppd"

case "$QUEUE" in ''|*[!a-zA-Z0-9_.-]*) echo 'error: invalid queue name' >&2; exit 1 ;; esac
[ "$(id -u)" -eq 0 ] || { echo "error: run sudo $0 $QUEUE" >&2; exit 1; }
[ -f "$DEST/.p1100-installed" ] && [ ! -L "$DEST" ] || { echo 'error: p1100 installation not found' >&2; exit 1; }
[ -f "$PPD" ] && [ ! -L "$PPD" ] && grep -Fq "*cupsFilter: \"application/vnd.cups-raster 100 $DEST/p1100-filter\"" "$PPD" || { echo 'error: queue is not using this p1100 driver' >&2; exit 1; }
[ -x "$APP/Contents/MacOS/P1100Utility" ] && [ -x "$APP/Contents/MacOS/p1100-inquire" ] && [ -f "$APP/Contents/Info.plist" ] || { echo 'error: build the utility first with ./native/build-app.sh' >&2; exit 1; }
[ ! -L "$DEST/P1100 Utility.app" ] || { echo 'error: refusing symlink utility destination' >&2; exit 1; }

TMP=$(mktemp -d "${TMPDIR:-/tmp}/p1100-utility.XXXXXXXX")
trap 'rm -rf "$TMP"' EXIT HUP INT TERM
awk '!/^\*APPrinterUtilityPath:/' "$PPD" > "$TMP/p1100.ppd"
printf '\n*APPrinterUtilityPath: "%s/P1100 Utility.app"\n' "$DEST" >> "$TMP/p1100.ppd"
if [ -x /usr/bin/cupstestppd ]; then
    /usr/bin/cupstestppd -I filters -I profiles "$TMP/p1100.ppd" || { echo 'error: updated PPD failed validation' >&2; exit 1; }
fi
/usr/bin/ditto "$APP" "$TMP/P1100 Utility.app"
rm -rf "$DEST/P1100 Utility.app"
/usr/bin/ditto "$TMP/P1100 Utility.app" "$DEST/P1100 Utility.app"
/usr/sbin/chown -R root:wheel "$DEST/P1100 Utility.app"
/usr/sbin/lpadmin -p "$QUEUE" -P "$TMP/p1100.ppd"
/usr/bin/install -m 0644 -o root -g wheel "$TMP/p1100.ppd" "$DEST/p1100.ppd"
echo "Updated $QUEUE with $DEST/P1100 Utility.app; the queue was not deleted."
