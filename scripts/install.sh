#!/bin/sh
# Install the native p1100 filter and create a CUPS queue for the HP LaserJet Professional P1100
# family. The device URI is inherited from this Mac's HP queue, which is a P1102.
#
# Design rules, intentionally:
#   - The existing HP installation is read but never written. The source PPD is only read.
#   - A new queue is created under a new name; the working (or broken) HP queue is left
#     exactly as it was, so this is reversible and side-by-side.
#   - Everything we add lives under /Library/Printers/p1100, one directory to delete.
#   - No SIP change, no system extension, no kernel extension, nothing disabled.
#
# `--dry-run` prints every command without running it and needs no privileges, and
# `--dest DIR --no-queue` runs every step except the queue into a scratch directory, which is
# how scripts/rehearse-install.sh exercises this script without root.

set -eu

QUEUE=HP_LaserJet_P1100_native
SOURCE_QUEUE=HP_LaserJet_Professional_P1102
DEST=/Library/Printers/p1100
REPO=$(cd "$(dirname "$0")/.." && pwd)
FILTER="$REPO/target/release/p1100-filter"
USBINK="$REPO/target/release/p1100-usbink"
ICON="$REPO/assets/p1100.icns"

# Apple's /usr/bin/python3 is behind the Xcode licence gate, so picking it blind is what made
# a dry run print "PPD generation would fail" and skip the artifact checks entirely - a check
# that reports ok while doing nothing. Prefer an interpreter that actually runs.
PYTHON=${PYTHON:-}
if [ -z "$PYTHON" ]; then
    for candidate in /opt/homebrew/bin/python3.13 /opt/homebrew/bin/python3.10 python3; do
        if "$candidate" -c 'pass' >/dev/null 2>&1; then PYTHON=$candidate; break; fi
    done
fi
PYTHON=${PYTHON:-python3}
DRY=0
FORCE=0
NOQUEUE=0
DEVICE=
MARKER=.p1100-installed

usage() {
    cat <<EOF
usage: $0 [--dry-run] [--force] [--queue NAME] [--source-queue NAME] [--device URI]
             [--icon FILE.icns] [--dest DIR] [--no-queue]

  --dry-run          print the commands instead of running them (no privileges needed)
  --force            replace an existing queue of the same name
  --queue NAME       queue to create            (default $QUEUE)
  --source-queue N   HP queue to inherit the device URI and PPD from (default $SOURCE_QUEUE)
  --device URI       use this device URI instead of reading it from the source queue
  --icon FILE        printer icon to install and point the PPD at (default $ICON)
  --dest DIR         where to install            (default $DEST)
  --no-queue         install the files and stop; no lpadmin, no root needed
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
        --dest) DEST=$2; shift ;;
        --no-queue) NOQUEUE=1 ;;
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
# The lines the generator is allowed to add, change or drop, as one pattern on a diff line.
# Anything else is printed and fails the install, which is what catches a generator
# regression that mangled the device description rather than the driver wiring.
PPD_ALLOWED='^[<>] \*(%%|PPD-Adobe|NickName|[a-zA-Z_@-]+\.Translation NickName/|cupsFilter: "application/vnd\.cups-(raster|command)|cupsCommands|APPrinterUtilityPath|APPrinterIconPath|OpenUI \*p1100Grey|OrderDependency: [0-9]+ [A-Za-z]+ \*p1100Grey|Defaultp1100Grey|p1100Grey |CloseUI: \*p1100Grey|[a-zA-Z_@-]+\.Translation p1100Grey/|[a-zA-Z_@-]+\.p1100Grey )'

ppd_diff() {
    diff - "$1" 2>/dev/null | grep -E '^[<>]' | grep -vE "$PPD_ALLOWED" | grep -vE '^[<>] *$' || true
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
    if ! iconutil -c iconset "$icon" -o "$work/p1100.iconset" >/dev/null 2>&1 \
       || [ ! -f "$work/p1100.iconset/icon_128x128.png" ]; then
        rm -rf "$work"
        die "$icon is not an icns macOS can read back; rebuild it with scripts/make-icns.py"
    fi
    rm -rf "$work"
    echo "icon: $icon"
}

# Two independent properties, because either one alone has already been not enough here.
#
# 1. Line 1 is *PPD-Adobe. A PPD whose first line is a comment is one CUPS cannot open, and it
#    fails quietly: cupsfilter falls back to its own conversion rules and produces a raster
#    anyway - 100 dpi, one bit per pixel, instead of the printer's 600 dpi grey. The generated
#    PPD was in exactly that state, so a queue installed from it would have printed the
#    fallback page. docs/testing.md carries the measurement.
# 2. The rest is judged by CUPS's own conformance test, which does ship with macOS
#    (/usr/bin/cupstestppd; this file claimed the opposite until that was checked). It reads
#    the parts the diff below cannot judge at all: the option UI, the translations and the
#    files the PPD references.
#
# The diff against the source stays, because cupstestppd never looks at the device data: it
# would pass a PPD whose paper sizes, resolutions and duplex settings had been mangled. So the
# property checked is still the one this document claims about the file - it differs from the
# source in exactly the intended ways, and in no other way.
check_ppd() {
    ppd=$1
    [ -s "$ppd" ] || die "the generated PPD is empty"

    head -1 "$ppd" | grep -q '^\*PPD-Adobe: "4\.3"' \
        || die "the generated PPD does not begin with *PPD-Adobe on line 1"
    grep -q "^\*cupsFilter: \"application/vnd.cups-raster 100 $DEST/p1100-filter\"" "$ppd" \
        || die "the generated PPD does not point its raster filter at $DEST/p1100-filter"
    grep -q "^\*cupsFilter: \"application/vnd.cups-command 100 $DEST/p1100-usbink\"" "$ppd" \
        || die "the generated PPD does not point its command filter at $DEST/p1100-usbink"
    grep -q '^\*cupsCommands: "ReportLevels"' "$ppd" \
        || die "the generated PPD does not offer ReportLevels"
    # The icon is the one line that names a file of ours outside this check's usual reach, so
    # it is asserted directly: the row would otherwise show HP's picture, or none.
    grep -q "^\*APPrinterIconPath: \"$DEST/p1100.icns\"$" "$ppd" \
        || die "the generated PPD does not point *APPrinterIconPath at $DEST/p1100.icns"
    if grep -E '^\*APPrinterIconPath:.*(/Library/Printers/hp|hp1100)' "$ppd" >/dev/null; then
        die "the generated PPD still points its icon into HP's installation"
    fi
    # Only filter lines matter here: a filter is a program, and none may come from HP's
    # installation, which cannot run on this machine at all.
    grep -E '^\*cupsFilter:.*(hpRaster\.bundle|/Library/Printers/hp)' "$ppd" \
        && die "the generated PPD still routes a filter into HP's installation"
    for gone in '^\*APPrinterUtilityPath'; do
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

    # CUPS's own conformance test. Its two file checks want ownership rather than a mode: a copy
    # of the filter at the mode this script installs (0755) in a temporary directory fails them,
    # while HP's own root-owned files pass. Only a run that has just installed root-owned copies
    # into /Library/Printers can satisfy them, so a dry run and a rehearsal into a scratch
    # directory turn them off with the documented -I flags, and the install that matters is
    # judged strictly. Everything else - the option UI, the translations - is judged in both.
    if [ -x /usr/bin/cupstestppd ]; then
        if [ "$DRY" = 1 ] || [ "$SYSTEM" = 0 ]; then
            report=$(/usr/bin/cupstestppd -I filters -I profiles "$ppd" 2>&1 || true)
        else
            report=$(/usr/bin/cupstestppd "$ppd" 2>&1 || true)
        fi
        fails=$(printf '%s\n' "$report" | grep -E '\*\*FAIL\*\*' || true)
        if [ -n "$fails" ]; then
            printf '%s\n' "$fails" >&2
            die "cupstestppd rejects the generated PPD (above)"
        fi
        echo "   cupstestppd: no conformance failures"
    else
        echo "   note: no /usr/bin/cupstestppd on this machine, so only the diff was checked"
    fi
    echo "   PPD checks out: only the driver wiring lines differ from $SRC_PPD"
}

# --- safety rails -----------------------------------------------------------

case "$DEST" in
    */hp|*/hp/*|/Library/Printers/PPDs*|/usr/libexec/cups*|/etc/cups*|/private/etc/cups*|/private/Library/Printers/PPDs*)
        die "refusing to install into $DEST: that is an Apple or HP location" ;;
esac
[ -n "$QUEUE" ] || die "empty queue name"
[ -d "$(dirname "$DEST")" ] || die "parent directory of --dest does not exist"
case "$(basename "$DEST")" in .|..) die "--dest must name a directory, not . or .." ;; esac
[ ! -L "$DEST" ] || die "refusing symlink destination: $DEST"
DEST_PARENT=$(cd -P "$(dirname "$DEST")" && pwd)
DEST="$DEST_PARENT/$(basename "$DEST")"
case "$DEST" in
    */hp|*/hp/*|/Library/Printers/PPDs*|/usr/libexec/cups*|/etc/cups*|/private/etc/cups*|/private/Library/Printers/PPDs*)
        die "refusing to install into $DEST: that is an Apple or HP location" ;;
esac

if [ -e "$DEST" ] || [ -L "$DEST" ]; then
    [ -d "$DEST" ] && [ ! -L "$DEST" ] || die "refusing a non-directory or symlink destination: $DEST"
    if [ ! -f "$DEST/$MARKER" ] && {
        [ ! -f "$DEST/p1100-filter" ] || [ ! -f "$DEST/p1100-usbink" ] || [ ! -f "$DEST/p1100.ppd" ] ||
        ! grep -Fq "*cupsFilter: \"application/vnd.cups-raster 100 $DEST/p1100-filter\"" "$DEST/p1100.ppd";
    }; then
        find "$DEST" -mindepth 1 -maxdepth 1 -print -quit | grep -q . \
            && die "refusing non-empty unowned destination $DEST"
    fi
fi
for f in "$MARKER" p1100-filter p1100-usbink p1100.ppd p1100.icns p1100-400.tones p1100-600.tones; do
    [ ! -L "$DEST/$f" ] || die "refusing known install file that is a symlink: $DEST/$f"
done

# Root is what writing under /Library/Printers costs, not what this script costs. --dest points
# it at a scratch directory instead, which is how scripts/rehearse-install.sh runs every step
# below except the queue without privileges and without going near anything Apple or HP owns.
SYSTEM=0
case "$DEST" in
    /Library/Printers/*) SYSTEM=1 ;;
esac
if [ "$DRY" = 0 ] && [ "$SYSTEM" = 1 ] && [ "$(id -u)" -ne 0 ]; then
    die "writing to $DEST and creating a queue need root; re-run with sudo
  (or rehearse the same steps in a scratch directory: --dest DIR --no-queue)"
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

if [ ! -x "$USBINK" ]; then
    die "$USBINK not found; build it with scripts/build-native.sh"
fi
case "$(/usr/bin/file -b "$USBINK")" in
    *arm64*) echo "command filter: $USBINK" ;;
    *) die "$USBINK is not an ARM64 binary" ;;
esac

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

if [ "$NOQUEUE" = 0 ] && [ -z "$DEVICE" ]; then
    DEVICE=$(lpstat -v "$SOURCE_QUEUE" 2>/dev/null | sed -n 's/^device for [^:]*: //p')
    [ -n "$DEVICE" ] || die "cannot read the device URI from queue '$SOURCE_QUEUE'; pass --device URI"
fi
[ "$NOQUEUE" = 1 ] || echo "device: $DEVICE"

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

if [ "$NOQUEUE" = 0 ]; then
    [ "$QUEUE" != "$SOURCE_QUEUE" ] || die "refusing to replace the source HP queue '$SOURCE_QUEUE'"
    QUEUE_EXISTS=0
    if lpstat -p "$QUEUE" >/dev/null 2>&1; then
        QUEUE_EXISTS=1
        [ "$FORCE" = 1 ] || die "queue '$QUEUE' already exists; use --force only for an installed p1100 queue"
        queue_ppd=/etc/cups/ppd/$QUEUE.ppd
        [ -f "$queue_ppd" ] && [ ! -L "$queue_ppd" ] && grep -Fq "*cupsFilter: \"application/vnd.cups-raster 100 $DEST/p1100-filter\"" "$queue_ppd" \
            || die "refusing to delete unrelated queue '$QUEUE'"
    fi
else
    QUEUE_EXISTS=0
fi

# --- act --------------------------------------------------------------------

echo
echo "1. installing the filter, the icon and the PPD into $DEST"
run mkdir -p "$DEST"
if [ "$DRY" = 0 ] && [ ! -f "$DEST/$MARKER" ]; then
    run /usr/bin/install -m 0644 /dev/null "$DEST/$MARKER"
fi
# Ownership belongs to the system location. A rehearsal into a scratch directory cannot set it -
# install(1) refuses - and does not need to, since nothing there is read by the print system.
OWNER=
if [ "$SYSTEM" = 1 ]; then
    OWNER="-o root -g wheel"
fi
run /usr/bin/install -m 0755 $OWNER "$FILTER" "$DEST/p1100-filter"
run /usr/bin/install -m 0755 $OWNER "$USBINK" "$DEST/p1100-usbink"
run /usr/bin/install -m 0644 $OWNER "$ICON" "$DEST/p1100.icns"
# The two tone tables the two-bit grey screening uses. They are HP data, so the installer
# reads them out of the HP driver on this machine rather than carrying them here. A machine
# without that driver still gets a working queue: only p1100Grey=Screened2 would fail, and
# the filter says so when it is asked for it.
if [ "$PYTHON_OK" = 1 ] && [ -f "$REPO/scripts/hp-tone-table.py" ]; then
    tones_tmp=$(mktemp -d -t p1100-tones) || die "cannot create a temporary directory"
    if "$PYTHON" "$REPO/scripts/hp-tone-table.py" --out "$tones_tmp" >/dev/null 2>&1 \
        && [ -s "$tones_tmp/sdtahs10.bin" ] && [ -s "$tones_tmp/sdtahr10.bin" ]; then
        run /usr/bin/install -m 0644 $OWNER "$tones_tmp/sdtahs10.bin" "$DEST/p1100-400.tones"
        run /usr/bin/install -m 0644 $OWNER "$tones_tmp/sdtahr10.bin" "$DEST/p1100-600.tones"
    else
        echo "note: no HP driver to read the tone tables from, so this queue will not offer"
        echo "      the two-bit grey screening. Everything else is unaffected."
    fi
    rm -f "$tones_tmp/sdtahs10.bin" "$tones_tmp/sdtahr10.bin"
    rmdir "$tones_tmp" 2>/dev/null || true
fi

echo
echo "   tone tables: an unprivileged copy is generated from the installed HP driver and"
echo "                installed as p1100-400.tones and p1100-600.tones, which is what"
echo "                p1100Grey=Screened2 reads."
if [ "$DRY" = 1 ]; then
    # Generate the PPD somewhere harmless and check it, so a dry run says something about
    # the artifact as well as about the commands.
    if [ "$PYTHON_OK" = 1 ]; then
        probe_ppd=$(mktemp -t p1100-ppd-check.ppd) || die "cannot create a temporary file"
        # A subshell with its own trap, so the file goes away however this ends - including the
        # check failing, which is how one was left behind after the check's first version died.
        (
            trap 'rm -f "$probe_ppd"' EXIT
            "$PYTHON" "$REPO/scripts/make-ppd.py" "$SRC_PPD" "$DEST/p1100-filter" "$probe_ppd" \
                --command-filter "$DEST/p1100-usbink" --icon "$DEST/p1100.icns" || exit 1
            check_ppd "$probe_ppd"
        ) || die "the PPD could not be generated or did not pass its check"
    fi
else
    run "$PYTHON" "$REPO/scripts/make-ppd.py" "$SRC_PPD" "$DEST/p1100-filter" "$DEST/p1100.ppd" \
        --command-filter "$DEST/p1100-usbink" --icon "$DEST/p1100.icns"
    # Before the queue is created, not after: a PPD CUPS cannot use must not reach lpadmin.
    check_ppd "$DEST/p1100.ppd"
fi

echo
if [ "$NOQUEUE" = 1 ]; then
    echo "2. no queue: --no-queue was given, so the files are in place and nothing points at them"
else
    echo "2. creating the queue '$QUEUE'"
    if [ "$FORCE" = 1 ] && [ "$QUEUE_EXISTS" = 1 ]; then
        run lpadmin -x "$QUEUE" 2>/dev/null || true
    fi
    run lpadmin -p "$QUEUE" -E -v "$DEVICE" -P "$DEST/p1100.ppd"
    run lpadmin -p "$QUEUE" -o printer-is-shared=false
    run cupsenable "$QUEUE"
    run cupsaccept "$QUEUE"
fi

echo
echo "done. the HP installation under /Library/Printers/hp was not touched."
echo
if [ "$NOQUEUE" = 1 ]; then
    echo "remove the files again with: $REPO/scripts/uninstall.sh --dest $DEST"
    exit 0
fi
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
