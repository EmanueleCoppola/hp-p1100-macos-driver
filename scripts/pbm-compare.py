#!/usr/bin/env python3
"""Compare two P4 (binary PBM) bitmaps by pixels, not by file bytes.

The reference decoder writes an equivalent but differently formatted PBM header
(padded, multi-line), so a byte comparison of whole files is meaningless. This
normalises the header and compares width, height and pixel data.

Exit status 0 when identical, 1 when they differ.

Usage: pbm-compare.py <expected.pbm> <actual.pbm>
"""

import sys


def read_p4(path):
    with open(path, 'rb') as f:
        d = f.read()
    if not d.startswith(b'P4'):
        raise SystemExit('not a P4 bitmap: %s' % path)
    i = 2
    vals = []
    while len(vals) < 2:
        while d[i:i + 1].isspace():
            i += 1
        if d[i:i + 1] == b'#':
            while d[i:i + 1] not in (b'\n', b''):
                i += 1
            continue
        j = i
        while not d[j:j + 1].isspace():
            j += 1
        vals.append(int(d[i:j]))
        i = j
    i += 1  # exactly one whitespace byte separates the header from the data
    w, h = vals
    stride = (w + 7) // 8
    return w, h, d[i:i + stride * h]


def main(argv):
    if len(argv) != 3:
        print(__doc__)
        return 2
    a = read_p4(argv[1])
    b = read_p4(argv[2])
    if a != b:
        print('MISMATCH: expected %dx%d %d bytes, got %dx%d %d bytes'
              % (a[0], a[1], len(a[2]), b[0], b[1], len(b[2])))
        return 1
    print('pixels identical (%dx%d, %d bytes)' % (a[0], a[1], len(a[2])))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))

