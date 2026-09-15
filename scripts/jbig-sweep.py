#!/usr/bin/env python3
"""Encode a spread of bitmaps with both implementations and require byte identity.

This is the check behind the claim in docs/jbig.md: for every case here, our encoder's
bi-level image entity is byte-for-byte the reference encoder's, in both the configuration
we ship (options 0x5c) and the plain one (options 0), which takes the other template and
skips typical prediction.

The cases are chosen to hit the parts that are easy to get wrong: blank and solid images,
uniform noise (worst case for an arithmetic coder), single rows and columns, odd widths,
exact stripe boundaries, and widths either side of the 128-pixel alignment.

Usage:
  scripts/jbig-oracle.sh fetch      # once
  scripts/jbig-sweep.py [--verbose]
"""

import argparse
import os
import random
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ORACLE = os.path.join(HERE, 'jbig-oracle.sh')
PROTOTYPE = os.path.join(HERE, 'jbig-encode.py')

OPTIONS = (0x5C, 0x00)


def write_pbm(path, width, height, rows):
    with open(path, 'wb') as f:
        f.write(b'P4\n%d %d\n' % (width, height))
        for row in rows:
            f.write(bytes(row))


def rows_of(width, height, ink):
    """Pack a predicate's ink into PBM rows, byte-padded to the width as stored."""
    stride = (width + 7) // 8
    out = []
    for y in range(height):
        row = bytearray(stride)
        for x in range(width):
            if ink(x, y):
                row[x >> 3] |= 0x80 >> (x & 7)
        out.append(row)
    return out


def cases():
    rng = random.Random(20260915)
    out = [
        ('blank 128x128', 128, 128, rows_of(128, 128, lambda x, y: False)),
        ('solid 128x128', 128, 128, rows_of(128, 128, lambda x, y: True)),
        ('single pixel', 1, 1, rows_of(1, 1, lambda x, y: True)),
        ('single blank pixel', 1, 1, rows_of(1, 1, lambda x, y: False)),
        ('one column, 200 rows', 1, 200, rows_of(1, 200, lambda x, y: y % 3 == 0)),
        ('one row', 200, 1, rows_of(200, 1, lambda x, y: x % 7 == 0)),
        ('8x2', 8, 2, rows_of(8, 2, lambda x, y: x % 2 == 0)),
        ('127x129', 127, 129, rows_of(127, 129, lambda x, y: (x + y) % 5 == 0)),
        ('129x130, just over a stripe', 129, 130, rows_of(129, 130, lambda x, y: (x * y) % 7 == 0)),
        ('128x256, one stripe boundary', 128, 256, rows_of(128, 256, lambda x, y: (y // 17) % 2 == 0)),
        ('256x300 checkerboard', 256, 300, rows_of(256, 300, lambda x, y: (x // 3 + y // 3) % 2 == 0)),
        ('gradient 300x260', 300, 260, rows_of(300, 260, lambda x, y: (x * 8) // 300 > (y * 8) // 260)),
        ('text-like 400x300', 400, 300, rows_of(400, 300, lambda x, y: 20 < y < 60 and (x // 6) % 2 == 0)),
        ('noise 1200x140', 1200, 140, rows_of(1200, 140, lambda x, y: rng.random() < 0.5)),
    ]
    for width in (7, 9, 63, 65, 127, 255, 1000):
        out.append(('noise %dx40' % width, width, 40,
                    rows_of(width, 40, lambda x, y: rng.random() < 0.35)))
    # 21 cases, every one encoded twice.
    return out


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--verbose', action='store_true', help='print every comparison')
    args = ap.parse_args(argv)

    probe = subprocess.run([ORACLE, 'which'], stdout=subprocess.PIPE)
    if not os.path.exists(os.path.join(probe.stdout.decode().strip(), 'jbigkit')):
        raise SystemExit('fetch the oracle first: scripts/jbig-oracle.sh fetch')

    work = tempfile.mkdtemp(prefix='p1102-jbig-sweep-')
    checked = 0
    failures = []
    for name, width, height, rows in cases():
        pbm = os.path.join(work, 'case.pbm')
        write_pbm(pbm, width, height, rows)
        for options in OPTIONS:
            ref = os.path.join(work, 'ref.jbg')
            ours = os.path.join(work, 'ours.jbg')
            subprocess.run([ORACLE, 'encode', pbm, ref, '-p', str(options)], check=True)
            subprocess.run([sys.executable, PROTOTYPE, pbm, ours, '--quiet',
                            '--options', str(options)], check=True)
            a = open(ref, 'rb').read()
            b = open(ours, 'rb').read()
            checked += 1
            if a != b:
                failures.append('%s at options 0x%02x: %d vs %d bytes' % (name, options, len(a), len(b)))
            elif args.verbose:
                print('  ok  %-28s options 0x%02x  %d bytes' % (name, options, len(a)))

    print('%d comparisons over %d cases, %d configurations'
          % (checked, len(cases()), len(OPTIONS)))
    for line in failures:
        print('  MISMATCH %s' % line)
    if failures:
        return 1
    print('every case is byte-identical to the reference encoder')
    # The scratch directory holds one bitmap and two encodings per case, and a passing run has
    # no reason to leave them behind; a failing one keeps them to look at.
    if not failures:
        shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
