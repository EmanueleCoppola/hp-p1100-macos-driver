#!/bin/sh
# Install the native p1102 filter and create a CUPS queue for the HP LaserJet Professional P1102.
#
# Design rules, intentionally:
#   - The existing HP installation is read but never written. The source PPD is only read.
#   - A new queue is created under a new name; the working (or broken) HP queue is left
#     exactly as it was, so this is reversible and side-by-side.
#   - Everything we add lives under /Library/Printers/p1102, one directory to delete.
#   - No SIP change, no system extension, no kernel extension, nothing disabled.
#
# `--dry-run` prints every command without running it and needs no privileges.

set -eu

QUEUE=HP_LaserJet_P1102_native
SOURCE_QUEUE=HP_LaserJet_Professional_P1102
DEST=/Library/Printers/p1102
REPO=$(cd "$(dirname "$0")/.." && pwd)
FILTER="$REPO/target/release/p1102-filter"
ICON="$REPO/assets/p1102.icns"
PYTHON=${PYTHON:-python3}
DRY=0
FORCE=0
DEVICE=

usage() {
    cat <<EOF
usage: $0 [--dry-run] [--force] [--queue NAME] [--source-queue NAME] [--device URI]
             [--icon FILE.icns]

  --dry-run          print the commands instead of running them (no privileges needed)
  --force            replace an existing queue of the same name
  --queue NAME       queue to create            (default $QUEUE)
  --source-queue N   HP queue to inherit the device URI and PPD from (default $SOURCE_QUEUE)
  --device URI       use this device URI instead of reading it from the source queue
  --icon FILE        printer icon to install and point the PPD at (default $ICON)
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY=1 ;;
        --force) FORCE=1 ;;
        --queue) QUEUE=$2; shift ;;
        --source-queue) SOURCE_QUEUE=$2; shift ;;
        --device) DEVICE=$2; shift ;;
        --icon) ICON=$2; shift ;;
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

# The lines of a diff against the source PPD that may legitimately change. The source arrives on
# stdin, so a gzipped one can be decompressed into it without a temporary file.
ppd_diff() {
    diff - "$1" 2>/dev/null | grep -E '^[<>]' | grep -vE \
        '^[<>] \*(%%|NickName|[a-zA-Z_@-]+\.Translation NickName/|cupsFilter: "application/vnd\.cups-(raster|command)|cupsCommands|APPrinterUtilityPath|APPrinterIconPath|p1102Grey)' \
        | grep -vE '^[<>] *$' || true
}

# --- validating a generated PPD ---------------------------------------------

# The icon is a picture the print system shows next to the queue, so a broken container is
# not a crash: it is a printer row that quietly falls back to a generic glyph. Hence the
# round trip through iconutil, which is the only reader that matters.
check_icon() {
    icon=$1
    [ -f "$icon" ] || die "the icon $icon is missing; build it with 'scripts/make-icns.py'"
    [ -s "$icon" ] || die "the icon $icon is empty"
    magic=$(head -c 4 "$icon")
    [ "$magic" = icns ] || die "$icon is not an icns container (it starts with '$magic')"
    work=$(mktemp -d) || die "cannot create a temporary directory"
    if ! iconutil -c iconset "$icon" -o "$work/p1102.iconset" >/dev/null 2>&1 \
       || [ ! -f "$work/p1102.iconset/icon_128x128.png" ]; then
        rm -rf "$work"
        die "$icon is not an icns macOS can read back; rebuild it with scripts/make-icns.py"
    fi
    rm -rf "$work"
    echo "icon: $icon"
}

# Two things CUPS itself cannot do here, both checked rather than assumed: macOS ships no
# cupstestppd, and cupsfilter falls back to its built-in conversion rules when a PPD declares
# nothing useful, so it produces a raster even from a deliberately empty PPD. So the property
# checked is the one the documentation actually claims about this file: it differs from the
# source PPD in exactly the intended ways, and in no other way at all. That is what would
# catch a generator regression that mangled the device data.
check_ppd() {
    ppd=$1
    [ -s "$ppd" ] || die "the generated PPD is empty"

    grep -q "^\*cupsFilter: \"application/vnd.cups-raster 100 $DEST/p1102-filter\"" "$ppd" \
        || die "the generated PPD does not point its raster filter at $DEST/p1102-filter"
    # The icon is the one line that names a file of ours outside this check's usual reach, so
    # it is asserted directly: the row would otherwise show HP's picture, or none.
    grep -q "^\*APPrinterIconPath: \"$DEST/p1102.icns\"$" "$ppd" \
        || die "the generated PPD does not point *APPrinterIconPath at $DEST/p1102.icns"
    if grep -E '^\*APPrinterIconPath:.*(/Library/Printers/hp|hp1100)' "$ppd" >/dev/null; then
        die "the generated PPD still points its icon into HP's installation"
    fi
    # Only filter lines matter here: a filter is a program, and none may come from HP's
    # installation, which cannot run on this machine at all.
    grep -E '^\*cupsFilter:.*(hpRaster\.bundle|/Library/Printers/hp)' "$ppd" \
        && die "the generated PPD still routes a filter into HP's installation"
    for gone in '^\*cupsCommands' '^\*APPrinterUtilityPath' '^\*cupsFilter: "application/vnd.cups-command'; do
        grep -q "$gone" "$ppd" && die "the generated PPD still carries a line matching $gone"
    done

    # Compare against the source. Only these line kinds may differ; anything else is a bug in
    # the generator, not a cosmetic difference. The source is piped in rather than copied to a
    # temporary file, because the source may be gzipped and a pipe keeps its bytes exactly -
    # and because a temporary file here would need cleaning up on every failure path.
    case "$SRC_PPD" in
        *.gz) decompress=gzcat ;;
        *)    decompress=cat ;;
    esac
    changes=$("$decompress" "$SRC_PPD" | diff - "$ppd" 2>/dev/null | grep -cE '^[<>]' || true)
    unexpected=$("$decompress" "$SRC_PPD" | ppd_diff "$ppd")
    # Fail closed: if the comparison produced nothing at all then it did not run - the source
    # could not be read, or the helper is missing - and an empty result would otherwise read
    # exactly like a clean one.
    [ "${changes:-0}" -gt 0 ] \
        || die "comparing the generated PPD with $SRC_PPD produced no differences at all, so the check did not run"
    if [ -n "$unexpected" ]; then
        echo "$unexpected" >&2
        die "the generated PPD changes lines it should not (above)"
    fi
    echo "   PPD checks out: only the driver wiring lines differ from $SRC_PPD"
}

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
        echo "      build it with './scripts/build-native.sh'"
    else
        die "$FILTER not found; build it first:
    scripts/build-native.sh
  (which uses Apple's linker when the Xcode licence is accepted and rust-lld with a
   rewritten stub SDK when it is not; see docs/testing.md)"
    fi
else
    arch=$(/usr/bin/file -b "$FILTER")
    case "$arch" in
        *arm64*) echo "filter: $FILTER ($arch)" ;;
        *) die "$FILTER is not an ARM64 binary: $arch" ;;
    esac
fi

# Same treatment for the icon: a real run stops before writing anything, a dry run says so
# and carries on, because the point of a dry run is to report rather than to fail.
if [ -f "$ICON" ] && [ -s "$ICON" ]; then
    check_icon "$ICON"
elif [ "$DRY" = 1 ]; then
    echo "note: $ICON does not exist or is empty; a real run would stop here."
    echo "      build it with './scripts/make-icns.py'"
else
    die "the icon $ICON is missing or empty; build it with:
    scripts/make-icns.py
  (which draws it and packs the .icns with iconutil and sips, both stock macOS)"
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
echo "1. installing the filter, the icon and the PPD into $DEST"
run mkdir -p "$DEST"
run /usr/bin/install -m 0755 -o root -g wheel "$FILTER" "$DEST/p1102-filter"
run /usr/bin/install -m 0644 -o root -g wheel "$ICON" "$DEST/p1102.icns"
# The two tone tables the two-bit grey screening uses. They are HP data, so the installer
# reads them out of the HP driver on this machine rather than carrying them here. A machine
# without that driver still gets a working queue: only p1102Grey=Screened2 would fail, and
# the filter says so when it is asked for it.
if [ "$PYTHON_OK" = 1 ] && [ -f "$REPO/scripts/hp-tone-table.py" ]; then
    tones_tmp=$(mktemp -d -t p1102-tones) || die "cannot create a temporary directory"
    if "$PYTHON" "$REPO/scripts/hp-tone-table.py" --out "$tones_tmp" >/dev/null 2>&1 \
        && [ -s "$tones_tmp/sdtahs10.bin" ] && [ -s "$tones_tmp/sdtahr10.bin" ]; then
        run /usr/bin/install -m 0644 -o root -g wheel "$tones_tmp/sdtahs10.bin" \
            "$DEST/p1102-400.tones"
        run /usr/bin/install -m 0644 -o root -g wheel "$tones_tmp/sdtahr10.bin" \
            "$DEST/p1102-600.tones"
    else
        echo "note: no HP driver to read the tone tables from, so this queue will not offer"
        echo "      the two-bit grey screening. Everything else is unaffected."
    fi
    rm -f "$tones_tmp/sdtahs10.bin" "$tones_tmp/sdtahr10.bin"
    rmdir "$tones_tmp" 2>/dev/null || true
fi

echo
echo "   tone tables: an unprivileged copy is generated from the installed HP driver and"
echo "                installed as p1102-400.tones and p1102-600.tones, which is what"
echo "                p1102Grey=Screened2 reads."
if [ "$DRY" = 1 ]; then
    # Generate the PPD somewhere harmless and check it, so a dry run says something about
    # the artifact as well as about the commands.
    if [ "$PYTHON_OK" = 1 ]; then
        probe_ppd=$(mktemp -t p1102-ppd-check.ppd) || die "cannot create a temporary file"
        # A subshell with its own trap, so the file goes away however this ends - including the
        # check failing, which is how one was left behind after the check's first version died.
        (
            trap 'rm -f "$probe_ppd"' EXIT
            "$PYTHON" "$REPO/scripts/make-ppd.py" "$SRC_PPD" "$DEST/p1102-filter" "$probe_ppd" \
                --icon "$DEST/p1102.icns" || exit 1
            check_ppd "$probe_ppd"
        ) || die "the PPD could not be generated or did not pass its check"
    fi
else
    run "$PYTHON" "$REPO/scripts/make-ppd.py" "$SRC_PPD" "$DEST/p1102-filter" "$DEST/p1102.ppd" \
        --icon "$DEST/p1102.icns"
    # Before the queue is created, not after: a PPD CUPS cannot use must not reach lpadmin.
    check_ppd "$DEST/p1102.ppd"
fi

echo
echo "2. creating the queue '$QUEUE'"
if [ "$FORCE" = 1 ]; then
    run lpadmin -x "$QUEUE" 2>/dev/null || true
fi
run lpadmin -p "$QUEUE" -E -v "$DEVICE" -P "$DEST/p1102.ppd"
run lpadmin -p "$QUEUE" -o printer-is-shared=false
run cupsenable "$QUEUE"
run cupsaccept "$QUEUE"

echo
echo "done. the HP installation under /Library/Printers/hp was not touched."
echo
echo "the queue's icon shows up next to its name in System Settings and in the print dialog."
echo "if the row still shows the old picture, the print system has it cached:"
echo "    sudo launchctl kickstart -k system/org.cups.cupsd"
echo
echo "try it:"
echo "    lp -d $QUEUE /path/to/file.pdf"
echo "    lpstat -o $QUEUE        # job state"
echo "    /usr/bin/log show --predicate 'process == \"cupsd\"' --last 5m   # job log"
echo
echo "remove it again with: sudo $REPO/scripts/uninstall.sh --queue $QUEUE"
