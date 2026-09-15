#!/usr/bin/env python3
"""Generate our PPD from the installed HP LaserJet P1100 PPD.

The HP PPD already describes the printer correctly (media sizes, resolutions, duplex,
imageable areas) and there is no reason to re-author that data. What has to change is
where the raster filter comes from, because HP's is an x86_64 binary that cannot run any
more.

This rewrites only the driver-wiring lines and leaves every device description byte for
byte, so the option set the printer needs is preserved exactly. Nothing is written over:
the source PPD is only read, and the output is a new file.

Rewritten:
  *cupsFilter: "application/vnd.cups-raster 100 <hp filter>"  -> our filter
  *APPrinterIconPath                                          -> our own .icns, with --icon
Dropped (no equivalent yet, so CUPS must not try to run them):
  *cupsFilter lines pointing at a command filter such as HP's usbink
  *cupsCommands, *APPrinterUtilityPath
Relabelled:
  *NickName gains a suffix so the queue is distinguishable in the UI.

Usage:
    python3 scripts/make-ppd.py <source.ppd[.gz]> <filter-path> <out.ppd>
                                [--suffix " (native ARM64)"] [--icon PATH.icns]
"""

import gzip
import sys

# HP filter basenames we must stop CUPS from invoking.
HP_FILTERS = ('rastertozjs', 'usbink', 'rasterToHPZJS', 'commandToHPZJS')


def read_maybe_gz(path):
    with open(path, 'rb') as f:
        head = f.read(2)
    if head == b'\x1f\x8b':
        with gzip.open(path, 'rb') as f:
            return f.read()
    with open(path, 'rb') as f:
        return f.read()


def rewrite(data, filter_path, suffix, icon_path=None):
    crlf = b'\r\n' in data
    eol = b'\r\n' if crlf else b'\n'
    lines = data.split(b'\n')
    # Normalise away a trailing CR so comparisons are simple; re-add on output.
    stripped = [ln[:-1] if ln.endswith(b'\r') else ln for ln in lines]

    out = []
    added = None
    icon_line = '*APPrinterIconPath: "%s"' % icon_path if icon_path else None
    icon_written = False
    old_nick = None
    new_nick = None
    for raw in stripped:
        line = raw.decode('latin1')
        low = line.lower()

        # Localised names are keyed by the NickName string, so they have to follow it or
        # the UI silently falls back to untranslated text.
        if line.startswith('*') and '.Translation NickName/' in line and new_nick is not None:
            head, _, tail = line.partition('.Translation NickName/')
            _, sep, rest = tail.partition(':')
            out.append('%s.Translation NickName/%s%s%s' % (head, new_nick, sep, rest))
            continue

        if line.startswith('*cupsCommands'):
            continue
        if line.startswith('*APPrinterUtilityPath'):
            continue
        # The icon is a picture the print system shows next to the queue, not a program, so
        # pointing it at our own file is safe - unlike a filter line, which must never point
        # into HP's tree. macOS reads this key and no other: the printer row in System
        # Settings matches the icns exactly (docs/installation.md has the measurement).
        if line.startswith('*APPrinterIconPath'):
            if icon_line:
                out.append(icon_line)
                icon_written = True
                continue
            out.append(line)
            continue
        if line.startswith('*cupsFilter'):
            value = line.split(':', 1)[1] if ':' in line else ''
            # The value is a quoted string: "<mime> <cost> <program>".
            value = value.strip().strip('"')
            mime = value.split()[0] if value.split() else ''
            if any(f in line for f in HP_FILTERS):
                if mime == 'application/vnd.cups-raster':
                    # Redirect the raster path at our filter, keeping the cost/priority.
                    added = '*cupsFilter: "application/vnd.cups-raster 100 %s"' % filter_path
                    out.append(added)
                    if icon_line and not icon_written:
                        # A source without an icon key at all still gets one, next to the
                        # wiring we just wrote rather than anywhere else.
                        out.append(icon_line)
                        icon_written = True
                # Otherwise it is a command filter: drop it.
                continue
            out.append(line)
            continue
        if line.startswith('*NickName'):
            old_nick = line.split(':', 1)[1].strip().strip('"') if ':' in line else ''
            new_nick = old_nick + suffix
            out.append(line.rstrip() + suffix)
            continue
        if line.startswith('*ShortNickName'):
            out.append(line)
            continue
        if low.startswith('*ppd-adobe') and not out:
            out.append(line)
            continue
        out.append(line)

    if added is None:
        raise SystemExit('error: no application/vnd.cups-raster filter line found in the source PPD')

    header = [
        '*%% Generated by p1102-drivers scripts/make-ppd.py from an installed HP PPD.',
        '*%% Only the driver-wiring lines differ from the source; device data is unchanged.',
    ]
    # The one option that is ours rather than HP's: the two-bit grey screening this driver can
    # do and HP's PPD has no way to ask for (docs/differences.md item 11). Mono1 is the default,
    # so a queue that sets nothing sends exactly what the reference driver sends.
    screening = [
        '*%%',
        '*%% Grey screening, added by p1102-drivers: Mono1 thresholds, Screened2 screens the',
        '*%% page through the printer-specific tone tables the installer puts beside the filter.',
        '*%% page through the printer-specific tone tables the installer puts beside the filter.',
        '*OpenUI *p1102Grey/Grey Screening: PickOne',
        '*OrderDependency: 10 AnySetup *p1102Grey',
        '*Defaultp1102Grey: Mono1',
        '*p1102Grey Mono1/One bit, thresholded: ""',
        '*p1102Grey Screened2/Two bits, screened: ""',
        '*CloseUI: *p1102Grey',
    ]
    body = header + screening + out
    result = eol.join(l.encode('latin1') for l in body)
    # Preserve whether the source ended with a newline rather than always adding one.
    if data.endswith(b'\n'):
        result += eol
    return result


def main(argv):
    args = [a for a in argv[1:] if not a.startswith('--')]
    suffix = ' (native ARM64)'
    if '--suffix' in argv:
        suffix = argv[argv.index('--suffix') + 1]
        args = [a for a in args if a != suffix]
    icon = None
    if '--icon' in argv:
        icon = argv[argv.index('--icon') + 1]
        args = [a for a in args if a != icon]
    if len(args) != 3:
        print(__doc__)
        return 2
    src, filter_path, dst = args
    data = read_maybe_gz(src)
    out = rewrite(data, filter_path, suffix, icon)
    with open(dst, 'wb') as f:
        f.write(out)
    print('wrote %s (%d bytes) from %s' % (dst, len(out), src))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
