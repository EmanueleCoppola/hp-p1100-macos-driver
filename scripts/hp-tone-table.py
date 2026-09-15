#!/usr/bin/env python3
"""Decode the grey tone tables HP's filter uses, out of its own binary.

Why this exists: HP's driver for this printer does not threshold grey the way this
repository's filter does (see docs/differences.md item 11). It converts each 8-bit grey pixel
to *two* bits per pixel through a 256-entry tone table whose entries are 12x12 clustered-dot
cells, and those tables are static data with symbols on them - so they can be read out of the
installed driver rather than guessed at. This script is how that reading is reproduced and
checked, and how the bytes are obtained if the two-bit path is ever implemented.

Which table is which: `zjs::JbigBandWriter::start` asks
`selectScreenTable(model name, vertical dpi, bits per pixel)` for a table. A vertical
resolution of 400 - which this PPD's default `cupsCompression 401` produces, because the
driver takes `compression & 0x3ffe` - selects `_sdtahs10`; anything else, including the
`600x600dpi` choice whose marker is 1, selects `_sdtahr10`. Both are 12x12, and the driver
packages each entry as `rows * (ceil(rows * 2 / 8) rounded up to 4)` bytes = 48, so 256
entries occupy 12288 bytes - which is exactly the distance between the two symbols.

Usage:
    scripts/hp-tone-table.py                 check the structure and print a summary
    scripts/hp-tone-table.py --grid 128      print the cell one grey level maps to
    scripts/hp-tone-table.py --out DIR       write one 12288-byte file per table
    scripts/hp-tone-table.py --selftest      run the same checks on a synthetic table

Exit status 0 when the tables decode as documented, 1 when they do not.
"""

import argparse
import importlib.util
import os
import shutil
import struct
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)

# The driver this project replaces, and the two tables this printer's PPD selects between.
HP_BINARY = ('/Library/Printers/hp/laserjet/p1100_1560_1600series/'
             'hpRaster.bundle/Contents/MacOS/rastertozjs')
TABLES = (('_sdtahs10', 400), ('_sdtahr10', 600))
ROWS = 12
COLS = 12


def load(name):
    """Import one of the sibling tools so its logic is reused rather than duplicated."""
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + '.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def entry_size(rows=ROWS, cols=COLS):
    """How many bytes the driver devotes to one grey level's cell."""
    row_bytes = (((rows * 2) // 8) + 3) & ~3
    return rows * row_bytes, row_bytes


def symbol_offsets(path):
    """File offsets of the tone-table symbols in a Mach-O binary, read with scripts/macho.py."""
    macho = load('macho')
    data = open(path, 'rb').read()
    out = {}
    for arch, off, size in macho.slices(data):
        m = macho.parse_mach(data, off, arch != 'thin')
        if not m:
            continue
        for s in macho.nlist_symbols(data, m, off):
            if s['name'] in dict(TABLES):
                out[s['name']] = s['value'] - 0x100000000
    return data, out


def decode(data, off, rows=ROWS, cols=COLS):
    """The table as 256 cells of rows x cols 2-bit values: cell[p][r][c]."""
    size, row_bytes = entry_size(rows, cols)
    cells = []
    for p in range(256):
        base = off + p * size
        entry = []
        for r in range(rows):
            row = []
            for c in range(cols):
                byte = data[base + r * row_bytes + c // 4]
                row.append((byte >> (6 - 2 * (c % 4))) & 3)
            entry.append(row)
        cells.append(entry)
    return cells


def synthetic(path):
    """A table with the documented shape, for checking the checks on a machine without HP."""
    size, row_bytes = entry_size()
    out = bytearray(size * 256)
    for p in range(256):
        level = min(3, (p * 4) // 256)
        for r in range(ROWS):
            for c in range(COLS):
                out[p * size + r * row_bytes + c // 4] |= level << (6 - 2 * (c % 4))
    with open(path, 'wb') as f:
        f.write(bytes(out))
    return out


def check(name, data, off, verbose=True):
    """Assert what docs/differences.md item 11 says about one table."""
    size, row_bytes = entry_size()
    end = off + size * 256
    if off < 0 or end > len(data):
        raise SystemExit('%s at 0x%x runs past the end of the file' % (name, off))
    cells = decode(data, off)
    problems = []
    if any(v for row in cells[0] for v in row):
        problems.append('entry 0 is not all black')
    if any(v != 3 for row in cells[255] for v in row):
        problems.append('entry 255 is not all white')
    # A tone table has to get lighter as the grey value rises: count the lightest cells.
    counts = [sum(1 for row in cell for v in row if v == 3) for cell in cells]
    drops = [p for p in range(1, 256) if counts[p] < counts[p - 1]]
    if drops:
        problems.append('%d grey level(s) are lighter than the one below them, first at %d'
                        % (len(drops), drops[0]))
    grid = ['%d' % counts[p] for p in (0, 32, 64, 96, 128, 160, 192, 224, 255)]
    if problems:
        raise SystemExit('%s: %s' % (name, '; '.join(problems)))
    if verbose:
        print('   %-10s offset 0x%x, %d entries of %d bytes (%d rows x %d cols, %d bytes per '
              'row), 12288 bytes' % (name, off, 256, size, ROWS, COLS, row_bytes))
        print('              lightest cells at grey 0/32/64/96/128/160/192/224/255: %s'
              % ' '.join(grid))
    return cells


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--binary', default=HP_BINARY, help='the HP filter to read (default: %s)'
                    % HP_BINARY)
    ap.add_argument('--grid', type=int, metavar='GREY',
                    help='print the 12x12 cell one grey level maps to')
    ap.add_argument('--out', metavar='DIR', help='write one file per table, 12288 bytes each')
    ap.add_argument('--selftest', action='store_true',
                    help='check a synthetic table instead of the installed driver')
    args = ap.parse_args(argv)

    if args.selftest:
        work = tempfile.mkdtemp(prefix='p1102-tone-')
        try:
            data = synthetic(os.path.join(work, 'synthetic.table'))
            print('selftest: a synthetic table with the documented shape')
            cells = check('synthetic', data, 0)
            if cells[128][0][0] not in (1, 2):
                raise SystemExit('selftest: grey 128 should be mid-level, not %d'
                                 % cells[128][0][0])
            print('   grey 128 maps to level %d at cell 0,0' % cells[128][0][0])
        finally:
            shutil.rmtree(work, ignore_errors=True)
        print('selftest passed')
        return 0

    if not os.path.exists(args.binary):
        print('no HP driver at %s: nothing to read (this check needs the installed driver)'
              % args.binary)
        return 0
    data, offsets = symbol_offsets(args.binary)
    print('tone tables in %s' % args.binary)
    missing = [n for n, _ in TABLES if n not in offsets]
    if missing:
        raise SystemExit('the driver does not export %s' % ', '.join(missing))
    cells = {}
    for name, dpi in TABLES:
        cells[name] = check(name, data, offsets[name])
    # The two tables are adjacent, which is what makes the 12288-byte entry count checkable
    # against something other than this script's own arithmetic.
    gap = offsets['_sdtahs10'] - offsets['_sdtahr10']
    if gap != entry_size()[0] * 256:
        raise SystemExit('_sdtahr10 and _sdtahs10 are %d bytes apart, not one table (12288)'
                         % gap)
    print('   the two tables are %d bytes apart, exactly one table: the entry size is right'
          % gap)
    if args.grid is not None:
        p = args.grid
        print('grey %d:' % p)
        for row in cells['_sdtahs10'][p]:
            print('   ' + ' '.join(str(v) for v in row))
    if args.out:
        os.makedirs(args.out, exist_ok=True)
        size = entry_size()[0]
        for name, _ in TABLES:
            path = os.path.join(args.out, name.lstrip('_') + '.bin')
            with open(path, 'wb') as f:
                f.write(data[offsets[name]:offsets[name] + size * 256])
            print('   wrote %s (%d bytes)' % (path, size * 256))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
