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
    scripts/hp-tone-table.py --convert RASTER --pages DIR
                                             screen a CUPS raster into 2-bit pages
    scripts/hp-tone-table.py --selftest      run the same checks on a synthetic table

The conversion is the driver's, not an invention: for each pixel the table is indexed by the
8-bit grey value, and inside that entry by the pixel's column group and its row, where the row
counts *down* as the page is walked down (the driver's line counter is the page height rounded
up to 12, less the line number, so the row is (-y) % 12). Each byte holds four 2-bit cells, one
per column in a group of four. What comes out is what HP's driver sends for this printer.

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


def level_of(table, grey, x, y):
    """One pixel's level, with the driver's indexing and its row phase.

    The row comes from the call's line counter, which is the page height rounded up to a
    multiple of 12 less the line number - so it is (-y) % 12, and the cell is walked upwards
    as the page is walked downwards. The column picks the byte by its group of four (only the
    first three bytes of a row are ever reached, because the counter wraps at 12) and the
    2-bit field inside it by its position in that group.
    """
    size, row_bytes = entry_size()
    byte = table[grey * size + ((-y) % ROWS) * row_bytes + ((x % COLS) // 4)]
    return (byte >> (6 - 2 * (x % 4))) & 3


def tiny_raster(path, width, height, rows, page_size_name=b'Letter'):
    """A one-page RaS3 raster: enough for the readers and writers in this repository."""
    cupsraster = load('cupsraster')
    header = bytearray(cupsraster.PAGE_HEADER_LEN)
    struct.pack_into('<I', header, cupsraster.CUPS_WIDTH, width)
    struct.pack_into('<I', header, cupsraster.CUPS_HEIGHT, height)
    struct.pack_into('<I', header, cupsraster.CUPS_BITS_PER_COLOR, 8)
    struct.pack_into('<I', header, cupsraster.CUPS_BITS_PER_PIXEL, 8)
    struct.pack_into('<I', header, cupsraster.CUPS_BYTES_PER_LINE, width)
    struct.pack_into('<I', header, cupsraster.CUPS_NUM_COLORS, 1)
    struct.pack_into('<II', header, cupsraster.HW_RESOLUTION, 600, 600)
    at = cupsraster.CUPS_PAGE_SIZE_NAME
    header[at:at + len(page_size_name)] = page_size_name
    with open(path, 'wb') as f:
        f.write(struct.pack('<I', cupsraster.SYNC_RASTER3))
        f.write(bytes(header))
        for row in rows:
            f.write(bytes(row))
    return path


def convert(raster_path, table, out_dir, stem='page'):
    """Screen each page of a CUPS raster into the two-bit page the driver would have sent.

    The page is written at the *aligned* width the encoder works in - the true width rounded
    up to a multiple of 128 - so the BIH's XD comes out the same as the one a one-bit job
    carries, and the padding pixels take the lightest level, which is what a zero padding bit
    means in the one-bit path.
    """
    cupsraster = load('cupsraster')
    os.makedirs(out_dir, exist_ok=True)
    size, row_bytes = entry_size()
    # One 256-entry translation table per (row of the cell, x % 12): the level a grey value
    # takes at that position. Screen a whole page with strided slice assignments rather than a
    # Python loop per pixel, which is the difference between seconds and minutes on a page.
    translations = {}
    for r in range(ROWS):
        per_column = []
        for c in range(COLS):
            base = r * row_bytes + c // 4
            shift = 6 - 2 * (c % 4)
            per_column.append(bytes((table[g * size + base] >> shift) & 3 for g in range(256)))
        translations[r] = per_column
    data = open(raster_path, 'rb').read()
    written = []
    for page, off, header, payload in cupsraster.walk(data):
        width, height = header['width'], header['height']
        bpl = header['bytes_per_line']
        aligned = (width + 127) & ~127
        grey = data[off + cupsraster.PAGE_HEADER_LEN:off + cupsraster.PAGE_HEADER_LEN + payload]
        path = os.path.join(out_dir, '%s-%d.pgm' % (stem, page + 1))
        row = bytearray([3]) * aligned
        groups = width // COLS
        data_row = bytearray(width)
        # Bilevel in the page, if not necessarily in the row padding: bytes(..) makes this a
        # C-level pass over the payload rather than a Python loop over thirty million pixels.
        bilevel = set(grey) <= {0, 255}
        with open(path, 'wb') as f:
            f.write(b'P5\n%d %d\n3\n' % (aligned, height))
            for y in range(height):
                src = grey[y * bpl:(y + 1) * bpl]
                per_column = translations[(-y) % ROWS]
                # Whole groups of COLS columns through the translation tables, and whatever
                # does not fill a group one pixel at a time.
                for c in range(min(COLS, width)):
                    if groups:
                        data_row[c:c + groups * COLS:COLS] = src[
                            c:c + groups * COLS:COLS].translate(per_column[c])
                for x in range(groups * COLS, width):
                    data_row[x] = level_of(table, src[x], x, y)
                if bilevel:
                    # A page of only 0 and 255 screens to only 0 and 3, so the two-bit page is
                    # the one-bit page with its levels scaled: that is what makes a one-bit and
                    # a two-bit job of the same page comparable on a printer, and it is worth
                    # failing on rather than noticing later.
                    for x in range(width):
                        want = 0 if src[x] < 128 else 3
                        if data_row[x] != want:
                            raise SystemExit(
                                '%s: grey %d at (%d, %d) screened to level %d, and a bi-level '
                                'page has to come out 0 or 3' % (raster_path, src[x], x, y,
                                                                 data_row[x]))
                row[:width] = data_row
                f.write(row)
                for x in range(width, aligned):
                    row[x] = 3
        written.append((path, aligned, height))
        if bilevel:
            print('   %s is bi-level, and the screening agrees with thresholding it'
                  % os.path.basename(path))
    return written


def synthetic_levels_table():
    """A table whose levels spell (grey + row + column) % 4, so the row phase is visible."""
    size, row_bytes = entry_size()
    out = bytearray(size * 256)
    for grey in range(256):
        for r in range(ROWS):
            for c in range(COLS):
                level = (grey + r + c) % 4
                out[grey * size + r * row_bytes + c // 4] |= level << (6 - 2 * (c % 4))
    return bytes(out)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--binary', default=HP_BINARY, help='the HP filter to read (default: %s)'
                    % HP_BINARY)
    ap.add_argument('--grid', type=int, metavar='GREY',
                    help='print the 12x12 cell one grey level maps to')
    ap.add_argument('--out', metavar='DIR', help='write one file per table, 12288 bytes each')
    ap.add_argument('--selftest', action='store_true',
                    help='check a synthetic table instead of the installed driver')
    ap.add_argument('--convert', metavar='RASTER',
                    help='screen a CUPS raster into two-bit pages (P5, maxval 3)')
    ap.add_argument('--pages', metavar='DIR', default='.',
                    help='where --convert writes its pages (default: the current directory)')
    ap.add_argument('--table', choices=('400', '600'), default='400',
                    help='tone table for --convert: the 400 dpi cell, which the PPD default '
                         'resolution selects, or the 600 dpi one')
    args = ap.parse_args(argv)

    if args.selftest:
        work = tempfile.mkdtemp(prefix='p1100-tone-')
        try:
            data = synthetic(os.path.join(work, 'synthetic.table'))
            print('selftest: a synthetic table with the documented shape')
            cells = check('synthetic', data, 0)
            if cells[128][0][0] not in (1, 2):
                raise SystemExit('selftest: grey 128 should be mid-level, not %d'
                                 % cells[128][0][0])
            print('   grey 128 maps to level %d at cell 0,0' % cells[128][0][0])
            levels = synthetic_levels_table()
            # The row phase counts down: same phase twelve lines later, a different one just
            # below. Everything else in the rule follows from that and the column arithmetic.
            assert level_of(levels, 10, 3, 4) == (10 + (-4) % ROWS + 3) % 4
            assert level_of(levels, 10, 3, 16) == level_of(levels, 10, 3, 4)
            assert level_of(levels, 10, 3, 5) != level_of(levels, 10, 3, 4)
            assert level_of(levels, 10, 15, 4) == level_of(levels, 10, 3, 4)
            samples = [[0, 64, 128, 192, 255, 255], [255, 192, 128, 64, 0, 0]]
            raster = tiny_raster(os.path.join(work, 'tiny.raster'), 6, len(samples), samples)
            page, width, height = convert(raster, levels, work, stem='tiny')[0]
            blob = open(page, 'rb').read()
            assert blob.startswith(b'P5\n128 2\n3\n'), blob[:16]
            pixels = blob[blob.index(b'3\n') + 2:]
            assert len(pixels) == 128 * 2
            # The fast path and the one-pixel-at-a-time rule have to agree everywhere.
            for y, line in enumerate(samples):
                for x, grey in enumerate(line):
                    assert pixels[y * width + x] == level_of(levels, grey, x, y), (x, y)
            assert pixels[127] == 3, 'the aligned padding should be the lightest level'
            print('   a 6x2 page screens to a %dx%d two-bit page, padding included'
                  % (width, height))
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
    if args.convert:
        name = '_sdtahs10' if args.table == '400' else '_sdtahr10'
        size = entry_size()[0]
        table = data[offsets[name]:offsets[name] + size * 256]
        for path, width, height in convert(args.convert, table, args.pages):
            print('   wrote %s (%dx%d, %d grey levels)'
                  % (path, width, height, 4))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
