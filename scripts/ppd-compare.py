#!/usr/bin/env python3
"""Drive CUPS with the PPD the installer writes, and compare the raster against HP's.

The CUPS raster is what the filter is handed, so if CUPS produces the same raster from our PPD
and from HP's for the same document and the same options, then every device fact the PPD
describes - paper sizes, imageable areas, resolutions, duplex, colour space - reaches the filter
identically, and the queue behaves the way HP's queue did.

This is also the check that catches the one failure this repository actually had, which is why
it exists rather than being folded into the dry run. A PPD whose first line is not *PPD-Adobe
is one CUPS cannot open, and CUPS does not say so: cupsfilter falls back to its built-in
conversion rules and produces a raster anyway, at 100 dpi and one bit per pixel instead of the
printer's 600 dpi grey. Compared against HP's PPD that fallback is unmistakable; looked at on
its own it reads as success. docs/testing.md has the measurement behind the numbers below.

The filter is named in the PPD but not run here: cupsfilter stops at the raster, which is the
filter's input. scripts/rehearse-job.py is what takes that raster the rest of the way, so the
two scripts meet in the middle.

Usage:
  scripts/ppd-compare.py [--keep DIR]
"""

import argparse
import filecmp
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
INSTALL_DIR = '/Library/Printers/p1100'
FIXTURE = os.path.join(REPO, 'fixtures/inputs/text-1p.txt')

# Both PPDs install.sh can generate from: the queue's, which is what this Mac has, and Apple's,
# which it falls back to on a machine with no HP queue. Both are checked, because "a machine
# without the HP driver still gets a working queue" is a claim in docs/installation.md, and this
# is what turns it into a check. The two describe the same device, so the same cases apply.
SOURCES = [
    ('/etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd', 'the installed queue PPD'),
    ('/Library/Printers/PPDs/Contents/Resources/hp1100.ppd.gz', "Apple's PPD, install.sh's fallback"),
]

# The options the generated PPD has to carry through unchanged, and whether each is known to
# change HP's own raster. That second field is what keeps the cases honest: an option that moves
# nothing in the raster tests nothing, so a case that stops moving it fails here instead of
# passing quietly.
#
# Two of them differ from the default by a handful of bytes, which is worth naming.
# Resolutions=600x600dpi changes one field, cupsCompression (401 to 1), and that field is the
# marker this driver reads to decide which tone table the page was screened with. MediaType
# changes only the media type string in the header. Both are exactly the kind of detail a
# generator regression would drop without anyone noticing.
#
# Options that only reach the printer's control language - duplex, density, jam recovery,
# economode, input slot - change the job rather than the raster, so they are not cases here;
# scripts/rehearse-job.py is where those are checked, through the filter.
CASES = [
    ('nothing set, the way a plain job arrives', [], False),
    ('A4 instead of the default Letter', ['PageSize=A4'], True),
    ('Legal', ['PageSize=Legal'], True),
    ('A5', ['PageSize=A5'], True),
    ('600x600dpi, which moves the compression marker',
     ['Resolutions=600x600dpi'], True),
    ('heavy media, which moves the media type in the header',
     ['MediaType=Heavy111-130g'], True),
]


def sources():
    found = [(path, label) for path, label in SOURCES if os.path.exists(path)]
    if not found:
        raise SystemExit('cannot find a source PPD: none of %s exists'
                         % ', '.join(path for path, _ in SOURCES))
    return found


def generate(dst, src):
    """Run the installer's generator exactly the way install.sh calls it."""
    cmd = [sys.executable, os.path.join(HERE, 'make-ppd.py'), src,
           INSTALL_DIR + '/p1100-filter', dst, '--icon', INSTALL_DIR + '/p1100.icns']
    proc = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        raise SystemExit('make-ppd.py failed:\n' + proc.stderr.decode('utf-8', 'replace'))
    return os.path.getsize(dst)


def cups_raster(ppd, options, dst):
    cmd = ['/usr/sbin/cupsfilter', '-m', 'application/vnd.cups-raster', '-p', ppd]
    for opt in options:
        cmd += ['-o', opt]
    cmd.append(FIXTURE)
    with open(dst, 'wb') as out:
        proc = subprocess.run(cmd, stdout=out, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        raise SystemExit('cupsfilter failed with %s:\n%s'
                         % (ppd, proc.stderr.decode('utf-8', 'replace')))
    return dst


def conformance(ppd):
    """CUPS's own conformance test, with its two file checks off, and only here.

    Nothing is installed under /Library/Printers/p1100 while this runs, so the filter and the
    icon it complains about are not there yet; install.sh runs the same test strictly against
    the files once they are, which is where their permissions get checked too.
    """
    proc = subprocess.run(['/usr/bin/cupstestppd', '-I', 'filters', '-I', 'profiles', ppd],
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    report = proc.stdout.decode('utf-8', 'replace')
    return [line.strip() for line in report.splitlines() if '**FAIL**' in line]


def same(a, b):
    return os.path.getsize(a) == os.path.getsize(b) and filecmp.cmp(a, b, shallow=False)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--keep', help='keep the rasters in this directory')
    args = ap.parse_args(argv)

    work = args.keep or tempfile.mkdtemp(prefix='p1100-ppd-')
    failures = 0
    compared = 0
    for n, (src, label) in enumerate(sources()):
        ours = os.path.join(work, 'p1100-%d.ppd' % n)
        size = generate(ours, src)
        with open(ours, 'rb') as f:
            first = f.readline().rstrip(b'\n')
        print('from %s (%s):' % (src, label))
        print('  %d bytes, line 1 is %s' % (size, first.decode('latin1')))
        if first != b'*PPD-Adobe: "4.3"':
            print('  a PPD whose first line is not *PPD-Adobe is one CUPS cannot open',
                  file=sys.stderr)
            failures += 1
            continue

        src_default = os.path.join(work, 'source-default-%d.raster' % n)
        cups_raster(src, [], src_default)
        for case, options, must_move in CASES:
            src_raster = os.path.join(work, 'source.raster')
            our_raster = os.path.join(work, 'ours.raster')
            cups_raster(src, options, src_raster)
            cups_raster(ours, options, our_raster)
            identical = same(src_raster, our_raster)
            moved = not same(src_raster, src_default)
            note = ''
            if must_move and not moved:
                note = "   <- no longer moves the source's raster, so this case tests nothing"
                failures += 1
            elif must_move:
                note = "   (moves the source's raster)"
            print('  %-52s %9d %s%s'
                  % (case, os.path.getsize(src_raster),
                     'identical' if identical else 'DIFFERENT', note))
            if not identical:
                failures += 1
            compared += 1

        fails = conformance(ours)
        if fails:
            print('  cupstestppd rejects the generated PPD:', file=sys.stderr)
            for line in fails:
                print('  ' + line, file=sys.stderr)
            failures += 1
        else:
            print('  cupstestppd: no conformance failures')

    if failures:
        print('%d of %d comparisons did not match' % (failures, compared), file=sys.stderr)
        return 1
    print('all %d comparisons produce the same raster as their source PPD' % compared)
    if args.keep:
        print('kept in %s' % work)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
