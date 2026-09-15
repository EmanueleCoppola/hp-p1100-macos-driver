#!/usr/bin/env python3
"""Read-only CUPS raster inspector using the layout measured on macOS 27.

This mirrors `crates/p1102-raster`, so the two can be cross-checked. Offsets are
`offsetof()` into CUPS's current `cups_page_header2_t`, which has no Magic/Version/
HeaderSize members: the sync word carries that role and is written once per stream.

    sync word   u32, once per stream (CUPS_RASTER_SYNC = 0x52615333 "RaS3")
    page n      cups_page_header2_t (1796 bytes), then bytes_per_line * height pixels

Every walk is validated: the decoded geometry must account for the remaining bytes
exactly, otherwise the tool says so instead of printing plausible nonsense.

Usage:
    python3 scripts/cupsraster.py dump <file> [maxpages]
    python3 scripts/cupsraster.py scan <file>
"""

import struct
import sys

# CUPS_RASTER_SYNC and friends, as stored by cupsRasterOpenIO (native byte order).
SYNC_RASTER3 = 0x52615333  # "RaS3", current format
SYNC_RASTER2 = 0x52615332  # "RaS2" / CUPS_RASTER_SYNC_PWG
SYNC_RASTER1 = 0x52615374  # "RaSt"
SYNC_APPLE = 0x554E4952    # "UNIR"
PAGE_HEADER_LEN = 1796

# offsetof() into cups_page_header2_t
MEDIA_CLASS = 0
MEDIA_COLOR = 64
MEDIA_TYPE = 128
OUTPUT_TYPE = 192
COLLATE = 264
DUPLEX = 272
HW_RESOLUTION = 276
IMAGING_BOUNDING_BOX = 284
MARGINS = 312
NUM_COPIES = 340
ORIENTATION = 344
PAGE_SIZE = 352
CUPS_WIDTH = 372
CUPS_HEIGHT = 376
CUPS_MEDIA_TYPE = 380
CUPS_BITS_PER_COLOR = 384
CUPS_BITS_PER_PIXEL = 388
CUPS_BYTES_PER_LINE = 392
CUPS_COLOR_ORDER = 396
CUPS_COLOR_SPACE = 400
CUPS_COMPRESSION = 404
CUPS_ROW_COUNT = 408
CUPS_ROW_FEED = 412
CUPS_ROW_STEP = 416
CUPS_NUM_COLORS = 420
CUPS_BORDERLESS_SCALING = 424
CUPS_PAGE_SIZE = 428
CUPS_IMAGING_BBOX = 436
CUPS_MARKER_TYPE = 1604
CUPS_RENDERING_INTENT = 1668
CUPS_PAGE_SIZE_NAME = 1732

COMPRESSION = {0: 'none', 1: 'RLE', 2: 'deflate', 3: 'gzip', 4: 'LZW', 5: 'JPEG', 6: 'PNG'}


def u32(d, o):
    return struct.unpack_from('<I', d, o)[0]


def f32(d, o):
    return struct.unpack_from('<f', d, o)[0]


def cstr(d, o, n):
    raw = d[o:o + n]
    return raw.split(b'\x00', 1)[0].decode('utf-8', 'replace')


def parse_header(d, o):
    return {
        'width': u32(d, o + CUPS_WIDTH),
        'height': u32(d, o + CUPS_HEIGHT),
        'bits_per_color': u32(d, o + CUPS_BITS_PER_COLOR),
        'bits_per_pixel': u32(d, o + CUPS_BITS_PER_PIXEL),
        'bytes_per_line': u32(d, o + CUPS_BYTES_PER_LINE),
        'num_colors': u32(d, o + CUPS_NUM_COLORS),
        'compression': u32(d, o + CUPS_COMPRESSION),
        'color_space': u32(d, o + CUPS_COLOR_SPACE),
        'resolution': (u32(d, o + HW_RESOLUTION), u32(d, o + HW_RESOLUTION + 4)),
        'margins': (u32(d, o + MARGINS), u32(d, o + MARGINS + 4)),
        'num_copies': u32(d, o + NUM_COPIES),
        'borderless': f32(d, o + CUPS_BORDERLESS_SCALING),
        'page_size': (f32(d, o + CUPS_PAGE_SIZE), f32(d, o + CUPS_PAGE_SIZE + 4)),
        'bbox': tuple(f32(d, o + CUPS_IMAGING_BBOX + 4 * i) for i in range(4)),
        'media_type': cstr(d, o + MEDIA_TYPE, 64),
        'cups_media_type': u32(d, o + CUPS_MEDIA_TYPE),
        'page_size_name': cstr(d, o + CUPS_PAGE_SIZE_NAME, 64),
    }


def walk(d):
    """Yield (page_index, header_offset, header, payload_len)."""
    if len(d) < 4:
        raise SystemExit('file too short')
    sync = u32(d, 0)
    if sync != SYNC_RASTER3:
        names = {SYNC_RASTER2: 'RaS2', SYNC_RASTER1: 'RaSt', SYNC_APPLE: 'UNIR'}
        raise SystemExit('sync word 0x%08x (%s); this reader handles RaS3 (0x%08x) only'
                         % (sync, names.get(sync, 'unknown'), SYNC_RASTER3))
    pos = 4
    page = 0
    while pos < len(d):
        if pos + PAGE_HEADER_LEN > len(d):
            print('  !! truncated header at %d' % pos)
            return
        h = parse_header(d, pos)
        # cupsCompression is informational: it comes from the PostScript page device
        # dictionary and libcups never consults it when reading pixels (page data is
        # always bytes_per_line * height raw bytes). Real cgpdftoraster output here
        # carries 401, which is not a CUPS code, so it is reported but not enforced.
        payload = h['bytes_per_line'] * h['height']
        end = pos + PAGE_HEADER_LEN + payload
        if end > len(d):
            print('  !! page %d geometry mismatch: header claims %d payload bytes '
                  'but only %d remain' % (page, payload, len(d) - pos - PAGE_HEADER_LEN))
            return
        yield page, pos, h, payload
        page += 1
        pos = end
    if pos != len(d):
        print('  !! walk ended at %d, file is %d bytes' % (pos, len(d)))


def dump(path, maxpages):
    d = open(path, 'rb').read()
    print('== %s (%d bytes)' % (path, len(d)))
    print('   sync word 0x%08x (RaS3), pages of [%d-byte cups_page_header2_t + w*h bytes]'
          % (u32(d, 0), PAGE_HEADER_LEN))
    n = 0
    for page, off, h, payload in walk(d):
        n += 1
        if n > maxpages:
            continue
        print('   -- page %d header @ %d --' % (page, off))
        print('      %dx%d px, %d bits/colour, %d bits/pixel, %d bytes/line, %d colour(s)'
              % (h['width'], h['height'], h['bits_per_color'], h['bits_per_pixel'],
                 h['bytes_per_line'], h['num_colors']))
        print('      payload %d bytes (= %d * %d)' % (payload, h['bytes_per_line'], h['height']))
        print('      media type %r, cupsMediaType %d, size name %r, page size %s pt, bbox %s, borderless %g'
              % (h['media_type'], h['cups_media_type'], h['page_size_name'],
                 tuple(round(v, 2) for v in h['page_size']),
                 tuple(round(v, 2) for v in h['bbox']), h['borderless']))
        print('      resolution %s dpi, margins %s pt, copies %d, compression %s'
              % (h['resolution'], h['margins'], h['num_copies'],
                 COMPRESSION.get(h['compression'], h['compression'])))
    print('   total pages: %d' % n)


def scan(path):
    d = open(path, 'rb').read()
    print('== %s (%d bytes)' % (path, len(d)))
    print('   sync word 0x%08x (stored native-endian, so it is not ASCII "RaS3")'
          % u32(d, 0))
    for page, off, h, payload in walk(d):
        print('   page %d: header @ %-10d %dx%d, payload %d'
              % (page, off, h['width'], h['height'], payload))


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    if argv[1] == 'dump':
        dump(argv[2], int(argv[3]) if len(argv) > 3 else 4)
    elif argv[1] == 'scan':
        scan(argv[2])
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
