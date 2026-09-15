#!/usr/bin/env python3
"""Build a complete, reference-correct ZjStream job for the HP LaserJet Professional P1100 family.
The reference-correctness was established against a P1102, the model this driver was developed
on and the one it is currently tested on.

This exists so the printer can be brought up on hardware *now*, before our own JBIG
encoder is written. It uses the reference encoder (scripts/jbig-oracle.sh) for the
compressed image - so the payload is known-good - and our own code for everything else:
the ZjStream chunk framing, the item sets and their order, and the PJL wrapper, all of
which mirror crates/p1100-protocol exactly.

The shipped filter now compresses with our own Rust encoder (crates/p1100-jbig) rather
than an external process. This tool stays useful for two things the filter cannot do:
producing a payload compressed by the *reference* implementation, so a printer rejection
can be attributed to the printer rather than to our encoder, and emitting the
uncompressed ZJT_RAW_IMAGE form for the same purpose.

Use it to answer the hardware questions that block everything else:

  * does the printer accept our PJL prologue and chunk framing at all?
  * is the START_PAGE item set and order right?
  * does ZJT_RAW_IMAGE work (--raw), or is JBIG genuinely required?
  * do the deterministic test pages print correctly, at the right size and orientation?

It is bring-up tooling, not part of the shipped filter.

Usage:
  scripts/make-test-job.py --pattern square -o square.zjs
  scripts/make-test-job.py --pattern text123 --page a4 -o a4.zjs
  scripts/make-test-job.py --pattern gray --raw -o gray-raw.zjs
  scripts/make-test-job.py --list

Patterns: blank, square, hline, vline, text123, gray, checker, border
"""

import argparse
import os
import struct
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# Geometry that matches what the printer's own PPD produces at 600 dpi, so a test job has
# exactly the page image a real job would have. These are the raster's dimensions, which are the
# PPD's *ImageableArea* with its 12 pt margins taken off, not its *PaperDimension*: for Legal
# that is 588 x 984 pt, 4900 x 8200 px. Measured from cgpdftoraster output, and held to it by
# scripts/bringup-compare.py - which is how the Legal height here was found to be the paper's
# 8100 rather than the 8200 a real job carries.
PAGES = {
    'letter': (4900, 6400, 612, 792, 1),   # DMPAPER_LETTER
    'a4': (4758, 6817, 595, 842, 9),       # DMPAPER_A4
    'legal': (4900, 8200, 612, 1008, 5),   # DMPAPER_LEGAL
}

DEFAULT_DPI = 600

# ZjStream opcodes and item tags, mirroring crates/p1100-protocol.
OP_START_DOC, OP_END_DOC, OP_START_PAGE, OP_END_PAGE = 0, 1, 2, 3
OP_JBIG_BIH, OP_JBIG_BID, OP_END_JBIG, OP_RAW_IMAGE = 4, 5, 6, 8
IT_DMCOLLATE, IT_DMDUPLEX, IT_DMPAPER = 1, 2, 3
IT_DMCOPIES, IT_DMDEFAULTSOURCE, IT_DMMEDIATYPE = 4, 5, 6
IT_NBIE, IT_RESX, IT_RESY = 7, 8, 9
IT_RASTER_X, IT_RASTER_Y = 12, 13
IT_VIDEO_BPP, IT_VIDEO_X, IT_VIDEO_Y = 16, 17, 18
IT_RET, IT_ECONOMODE = 22, 23

CHUNK_HEADER = 16
ITEM_UINT32 = 12
CHUNK_SIGNATURE = 0x5A5A
STREAM_SIGNATURE = b'JZJZ'
JBIG_BIH_LEN = 20
JBIG_BID_MAX = 65536
# What the reference encoder adds to the end of the final image chunk, beyond aligning it
# to 4 bytes; its default for the -X option.
JBIG_BID_EXTRA_PAD = 16
UEL = b'\x1b%-12345X'


def align128(v):
    return (v + 127) & ~127


# --------------------------------------------------------------------------- bitmap

def make_bitmap(pattern, w, h):
    """1 bpp bitmap, one bit per pixel, set bit = ink. Rows are align128(w)/8 bytes."""
    stride = align128(w) // 8
    bm = bytearray(stride * h)

    def plot(x, y):
        if 0 <= x < w and 0 <= y < h:
            bm[y * stride + (x >> 3)] |= 0x80 >> (x & 7)

    if pattern == 'blank':
        pass
    elif pattern == 'square':
        # 1 inch square, 1 inch in from the top-left corner
        x0, y0 = 600, 600
        for y in range(y0, y0 + 600):
            for x in range(x0, x0 + 600):
                plot(x, y)
    elif pattern == 'hline':
        for x in range(0, w):
            for y in range(h // 2, h // 2 + 8):
                plot(x, y)
    elif pattern == 'vline':
        for y in range(0, h):
            for x in range(w // 2, w // 2 + 8):
                plot(x, y)
    elif pattern == 'border':
        for x in range(0, w):
            plot(x, 50)
            plot(x, h - 51)
        for y in range(0, h):
            plot(50, y)
            plot(w - 51, y)
    elif pattern == 'checker':
        for y in range(0, h):
            for x in range(0, w):
                if (((x >> 5) + (y >> 5)) & 1) == 0:
                    plot(x, y)
    elif pattern == 'gray':
        # Eight bands of increasing ink density (1/8 .. 8/8), matching the Rust generator
        # in crates/p1100-job so both tools produce the same page.
        bands = 8
        band_h = h // bands
        for b in range(bands):
            step = b + 1
            for y in range(b * band_h, min((b + 1) * band_h, h)):
                for x in range(0, w):
                    if (x % 8) < step:
                        plot(x, y)
    elif pattern == 'text123':
        # "TEST 123" from a 5x7 bitmap font, scaled up, near the top-left.
        font = {
            'T': ['11111', '00100', '00100', '00100', '00100', '00100', '00100'],
            'E': ['11111', '10000', '10000', '11110', '10000', '10000', '11111'],
            'S': ['01111', '10000', '10000', '01110', '00001', '00001', '11110'],
            '1': ['00100', '01100', '00100', '00100', '00100', '00100', '01110'],
            '2': ['01110', '10001', '00001', '00010', '00100', '01000', '11111'],
            '3': ['11110', '00001', '00001', '01110', '00001', '00001', '11110'],
            ' ': ['00000'] * 7,
        }
        text = 'TEST 123'
        scale = 24
        x0, y0 = 300, 300
        x = x0
        for ch in text:
            glyph = font.get(ch)
            if glyph:
                for gy, row in enumerate(glyph):
                    for gx, c in enumerate(row):
                        if c == '1':
                            for dy in range(scale):
                                for dx in range(scale):
                                    plot(x + gx * scale + dx, y0 + gy * scale + dy)
            x += 6 * scale
    else:
        raise SystemExit('unknown pattern: %s' % pattern)

    return bm, stride


def write_pbm(path, bm, w, h, stride):
    with open(path, 'wb') as f:
        f.write(b'P4\n%d %d\n' % (w, h))
        # PBM rows are packed with no padding beyond the row itself; since our stride is a
        # multiple of 16 bytes it is already 8-bit aligned, so the rows are identical.
        f.write(bytes(bm))


# --------------------------------------------------------------------------- zjstream

class Zjs(object):
    def __init__(self):
        self.out = bytearray()

    def raw(self, data):
        self.out += data

    def chunk(self, ty, items=(), payload=b'', reserved=None):
        item_bytes = bytearray()
        for tag, value in items:
            item_bytes += struct.pack('>I', ITEM_UINT32)
            item_bytes += struct.pack('>H', tag)
            item_bytes += bytes([1, 0])          # ZJIT_UINT32, param
            item_bytes += struct.pack('>I', value)
        if reserved is None:
            reserved = len(item_bytes)
        size = CHUNK_HEADER + len(item_bytes) + len(payload)
        self.out += struct.pack('>IIIHH', size, ty, len(items), reserved, CHUNK_SIGNATURE)
        self.out += item_bytes
        self.out += payload

    def item_count(self, items):
        return len(items)


def pjl_prologue(density, econo, timestamp):
    lines = [
        UEL + b'@PJL JOB\n',
        b'@PJL SET JAMRECOVERY=OFF\n',
        b'@PJL SET DENSITY=%d\n' % density,
        b'@PJL SET ECONOMODE=' + (b'ON' if econo else b'OFF') + b'\n',
        b'@PJL SET RET=MEDIUM\n',
        b'@PJL INFO STATUS\n',
        b'@PJL USTATUS DEVICE = ON\n',
        b'@PJL USTATUS JOB = ON\n',
        b'@PJL USTATUS PAGE = ON\n',
        b'@PJL USTATUS TIMED = 30\n',
        b'@PJL SET JOBATTR="JobAttr4=%s"' % timestamp.encode(),
        b'\x00',
        UEL,
    ]
    return b''.join(lines)


def pjl_epilogue():
    return UEL + b'@PJL EOJ\n' + UEL


def build_job(bie, width, height, dpi, paper, opts):
    """Mirror crates/p1100-job exactly: item sets, order and chunk sequence.

    The item sets matter for bring-up: whatever this emits is what the printer sees, so it
    has to match what the Rust filter emits for the same options. In particular EconoMode
    adds a page item, and duplexing adds a DMDUPLEX item to the document header, the image
    end and the page end (and removes the "off" item from the document header).
    """
    enc = Zjs()
    if opts.pjl:
        enc.raw(pjl_prologue(opts.density, opts.econo, opts.timestamp))
    enc.raw(STREAM_SIGNATURE)

    duplexing = opts.duplex in (2, 3)
    doc_items = [(IT_DMCOLLATE, 0)]
    if not duplexing:
        doc_items.append((IT_DMDUPLEX, 1))
    enc.chunk(OP_START_DOC, doc_items)

    page_items = [
        # 1 bpp, so video X equals the pixel width
    ]
    if duplexing:
        page_items.append((IT_DMDUPLEX, opts.duplex))
    if opts.econo:
        page_items.append((IT_ECONOMODE, 1))
    page_items += [
        (IT_VIDEO_X, width),
        (IT_VIDEO_Y, height),
        (IT_VIDEO_BPP, 1),
        (IT_RASTER_X, width),     # true width, not the 128-aligned one
        (IT_RASTER_Y, height),
        (IT_NBIE, 1),
        (IT_RESX, dpi),
        (IT_RESY, dpi),
        (IT_RET, 1),
        (IT_DMDEFAULTSOURCE, 1),
        (IT_DMCOPIES, 1),
        (IT_DMPAPER, paper),
        (IT_DMMEDIATYPE, 1),
    ]
    enc.chunk(OP_START_PAGE, page_items)

    if bie is None:
        # Raw, uncompressed: the whole bitmap in one chunk. Layout unconfirmed.
        enc.chunk(OP_RAW_IMAGE, (), opts.bitmap)
    else:
        enc.chunk(OP_JBIG_BIH, (), bie[:JBIG_BIH_LEN])
        body = bie[JBIG_BIH_LEN:]
        chunks = [body[i:i + JBIG_BID_MAX] for i in range(0, len(body), JBIG_BID_MAX)]
        if not chunks:
            chunks = [b'']
        for i, c in enumerate(chunks):
            if i == len(chunks) - 1:
                # The reference encoder's default for its -X option: 16 zero bytes on top
                # of rounding the chunk up to 4 bytes. Inert, but it keeps this job
                # comparable byte for byte with one the reference produces.
                pad = JBIG_BID_EXTRA_PAD + (4 - len(c) % 4) % 4
                c = c + b'\x00' * pad
            enc.chunk(OP_JBIG_BID, (), c)

    end_items = [(IT_DMDUPLEX, opts.duplex)] if duplexing else ()
    enc.chunk(OP_END_JBIG, end_items, b'', reserved=None if duplexing else 0)
    enc.chunk(OP_END_PAGE, end_items, b'', reserved=None if duplexing else 0)
    enc.chunk(OP_END_DOC, (), b'', reserved=0)
    if opts.pjl:
        enc.raw(pjl_epilogue())
    return bytes(enc.out)


def walk(job):
    """Parse a job back into chunks; used by --verify."""
    start = job.find(STREAM_SIGNATURE)
    if start < 0:
        raise SystemExit('no JZJZ signature in the job')
    pos = start + len(STREAM_SIGNATURE)
    end = job.find(UEL + b'@PJL EOJ', pos)
    if end < 0:
        end = len(job)
    chunks = []
    while pos < end:
        size, ty, items, reserved, sig = struct.unpack_from('>IIIHH', job, pos)
        if sig != CHUNK_SIGNATURE:
            raise SystemExit('bad chunk signature at %d: %#06x' % (pos, sig))
        items_data = bytearray()
        ip = pos + CHUNK_HEADER
        for _ in range(items):
            isize, tag, itype, param = struct.unpack_from('>IHBB', job, ip)
            items_data += job[ip:ip + isize]
            ip += isize
        payload = job[ip:pos + size]
        chunks.append({'type': ty, 'items': items, 'reserved': reserved,
                       'item_bytes': len(items_data), 'payload': payload,
                       'size': size})
        if size < CHUNK_HEADER:
            raise SystemExit('chunk size too small at %d' % pos)
        pos += size
    return chunks


def verify(job, bie, opts):
    chunks = walk(job)
    types = [c['type'] for c in chunks]
    jbig = bie is not None
    want = [OP_START_DOC, OP_START_PAGE,
            OP_JBIG_BIH if jbig else OP_RAW_IMAGE]
    if jbig:
        want += [OP_JBIG_BID]
    want += [OP_END_JBIG, OP_END_PAGE, OP_END_DOC]
    if types[:3] != want[:3] or types[-3:] != want[-3:]:
        raise SystemExit('unexpected chunk sequence: %s' % types)
    if OP_END_JBIG not in types or OP_END_PAGE not in types:
        raise SystemExit('missing END_JBIG / END_PAGE')
    for c in chunks:
        if c['items'] and c['reserved'] != c['item_bytes']:
            raise SystemExit('reserved != item bytes for chunk type %d' % c['type'])
        if c['size'] != CHUNK_HEADER + c['item_bytes'] + len(c['payload']):
            raise SystemExit('chunk size does not add up for type %d' % c['type'])
    if jbig:
        reassembled = b''.join(c['payload'] for c in chunks if c['type'] in (OP_JBIG_BIH, OP_JBIG_BID))
        # the last BID chunk carries padding that is not part of the BIE
        trimmed = reassembled[:len(bie)]
        if trimmed != bie:
            raise SystemExit('reassembled BIE differs from the encoder output')
        print('verify: chunk sequence ok, %d BID chunk(s), BIE %d bytes round-tripped'
              % (sum(1 for t in types if t == OP_JBIG_BID), len(bie)))
    else:
        raw = [c for c in chunks if c['type'] == OP_RAW_IMAGE][0]
        if raw['payload'] != opts.bitmap:
            raise SystemExit('raw payload differs from the bitmap')
        print('verify: chunk sequence ok, raw payload %d bytes' % len(raw['payload']))


# --------------------------------------------------------------------------- main

def main(argv):
    ap = argparse.ArgumentParser(add_help=True, description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--pattern', default='text123')
    ap.add_argument('--page', default='letter', choices=sorted(PAGES))
    ap.add_argument('--dpi', type=int, default=DEFAULT_DPI)
    ap.add_argument('--density', type=int, default=3)
    ap.add_argument('--economode', action='store_true')
    ap.add_argument('--duplex', default='off', choices=['off', 'long', 'short'],
                    help='emit the ZJI_DMDUPLEX items for two-sided printing')
    ap.add_argument('--no-pjl', action='store_true')
    ap.add_argument('--raw', action='store_true',
                    help='emit ZJT_RAW_IMAGE instead of JBIG (layout unconfirmed)')
    ap.add_argument('--timestamp', default='19700101000000')
    ap.add_argument('--keep-pbm', metavar='PATH', help='also write the bitmap as PBM')
    ap.add_argument('--verify', action='store_true')
    ap.add_argument('--list', action='store_true')
    ap.add_argument('-o', '--out', default='test-page.zjs')
    opts = ap.parse_args(argv)

    if opts.list:
        print('patterns: blank square hline vline text123 gray checker border')
        print('pages:    %s' % ' '.join(sorted(PAGES)))
        return 0

    w, h, _, _, paper = PAGES[opts.page]
    # Normalise the flag names the job builder reads.
    opts.pjl = not opts.no_pjl
    opts.econo = opts.economode
    opts.duplex = {'off': 1, 'long': 2, 'short': 3}[opts.duplex]
    if opts.dpi != DEFAULT_DPI:
        # Scale the measured 600 dpi geometry rather than recomputing from the PPD.
        f = float(opts.dpi) / DEFAULT_DPI
        w, h = int(round(w * f)), int(round(h * f))

    bm, stride = make_bitmap(opts.pattern, w, h)
    opts.bitmap = bytes(bm)
    print('pattern %s, %s at %d dpi -> %dx%d px, %d bytes of 1bpp (stride %d)'
          % (opts.pattern, opts.page, opts.dpi, w, h, len(bm), stride))

    bie = None
    if not opts.raw:
        pbm = opts.keep_pbm or os.path.join(os.environ.get('TMPDIR', '/tmp'), 'p1100-test.pbm')
        write_pbm(pbm, opts.bitmap, align128(w), h, stride)
        jbg = pbm + '.jbg'
        subprocess.check_call(['%s/jbig-oracle.sh' % HERE, 'encode', pbm, jbg])
        with open(jbg, 'rb') as f:
            bie = f.read()
        if not opts.keep_pbm:
            os.unlink(pbm)
        os.unlink(jbg)
        print('JBIG image: %d bytes (BIH %d + body %d), reference encoder'
              % (len(bie), JBIG_BIH_LEN, len(bie) - JBIG_BIH_LEN))
    else:
        print('raw image: %d bytes (ZJT_RAW_IMAGE, unconfirmed layout)' % len(opts.bitmap))

    job = build_job(bie, w, h, opts.dpi, paper, opts)
    with open(opts.out, 'wb') as f:
        f.write(job)
    print('wrote %s (%d bytes, pjl=%s)' % (opts.out, len(job), not opts.no_pjl))

    if opts.verify:
        verify(job, bie, opts)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
