#!/usr/bin/env python3
"""Check that the filter's peak memory does not grow with the job size.

The filter streams: it reads the raster a page at a time and writes each page out as soon as it
is encoded, so peak memory should be one page of raster plus one page's compressed form, no
matter how many pages the job has. That is the property `docs/architecture.md` claims with
numbers, and this is the check behind it.

Measured with `/usr/bin/time -l`, because that is what the documented numbers came from. The
bar is deliberately loose - a factor of 1.25 between a one-page job and a ten-page one - since
the point is to catch a return to buffering the whole job, which shows up as a factor of ten,
not to pin the allocator down to a page.

Usage:
  scripts/memory-check.py [--pages N] [--verbose]
"""

import argparse
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
FILTER = os.path.join(REPO, 'target/release/p1100-filter')
PPD = '/etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd'


def make_raster(path, pages):
    text = os.path.join(os.path.dirname(path), 'pages.txt')
    with open(text, 'w') as f:
        for n in range(pages):
            f.write('PAGE %d of the streaming check\n\f' % (n + 1))
    with open(path, 'wb') as out:
        proc = subprocess.run(['/usr/sbin/cupsfilter', '-m', 'application/vnd.cups-raster',
                               '-p', PPD, text], stdout=out, stderr=subprocess.DEVNULL)
    if proc.returncode != 0:
        raise SystemExit('cupsfilter failed; cannot build a %d-page raster' % pages)
    return os.path.getsize(path)


def peak_rss(raster):
    """Peak resident set size of one filter run, in bytes, from /usr/bin/time -l."""
    log = raster + '.time'
    with open(log, 'w') as err, open(os.devnull, 'wb') as sink:
        proc = subprocess.run(['/usr/bin/time', '-l', FILTER, '1', 'manu', 'memory', '1', '',
                               raster], stdout=sink, stderr=err)
    if proc.returncode != 0:
        raise SystemExit('the filter failed on %s' % raster)
    for line in open(log):
        if 'maximum resident set size' in line:
            return int(line.split()[0])
    raise SystemExit('no resident set size in the output of /usr/bin/time -l')


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--pages', type=int, default=10, help='pages in the larger job')
    ap.add_argument('--verbose', action='store_true')
    args = ap.parse_args(argv)

    if not os.path.exists(FILTER):
        raise SystemExit('build the filter first: ./scripts/build-native.sh')

    work = tempfile.mkdtemp(prefix='p1100-memory-')
    try:
        small = os.path.join(work, 'one.raster')
        large = os.path.join(work, 'many.raster')
        small_bytes = make_raster(small, 1)
        large_bytes = make_raster(large, args.pages)
        small_rss = peak_rss(small)
        large_rss = peak_rss(large)
    finally:
        for name in sorted(os.listdir(work)):
            os.unlink(os.path.join(work, name))
        os.rmdir(work)

    ratio = large_rss / small_rss
    print('  %d pages (%d MB of raster): peak %d MB' % (1, small_bytes // 1000000,
                                                        small_rss // 1000000))
    print('  %d pages (%d MB of raster): peak %d MB' % (args.pages, large_bytes // 1000000,
                                                        large_rss // 1000000))
    print('  ratio %.2f' % ratio)
    if ratio >= 1.25:
        print('  FAIL: peak memory grows with the job, so the filter is buffering again')
        return 1
    print('  peak memory is flat in the job size')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
