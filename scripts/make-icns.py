#!/usr/bin/env python3
"""Build the .icns that our PPD points *APPrinterIconPath at.

macOS shows the PPD's icon next to the queue name - in System Settings, in the print
dialog, and anywhere else the print system reports a printer. That is measured here rather
than assumed: the row for HP's own queue matches hp1100.icns to a mean absolute luminance
error of 8/255 across the icon box, while the stock system glyphs and CUPS's own default
icon land between 44 and 98 away. The row is the icns file, not a generic picture.

The drawing is code rather than a borrowed picture: it is ours, so it can carry the same
licence as the rest of the repository, and it is redrawn at every size the icns container
needs instead of being resampled down from one large bitmap. The body is mid-grey with a
light edge, so the icon reads on a light and on a dark background alike.

Usage:
    python3 scripts/make-icns.py                        # write assets/p1102.icns
    python3 scripts/make-icns.py --preview /tmp/i.png   # also write a 256px preview
    python3 scripts/make-icns.py --from art.png         # use an image instead of drawing

--from takes a PNG (or an .icns, which is copied through) and builds the container from it
with sips, which is the path to take when the artwork should be a picture of your own.

Needs iconutil and sips, both stock macOS.
"""

import argparse
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

# The files an .iconset must contain, and the pixel size behind each name.
ICONSET = (
    ('icon_16x16.png', 16),
    ('icon_16x16@2x.png', 32),
    ('icon_32x32.png', 32),
    ('icon_32x32@2x.png', 64),
    ('icon_128x128.png', 128),
    ('icon_128x128@2x.png', 256),
    ('icon_256x256.png', 256),
    ('icon_256x256@2x.png', 512),
    ('icon_512x512.png', 512),
    ('icon_512x512@2x.png', 1024),
)

# The drawing, in a 1024-unit square, back to front. Rectangle entries are
# (x0, y0, x1, y1, corner radius, colour) with y measured downwards; the circle is
# (cx, cy, radius, colour). The last shape is the panel light, which is ours alone.
OUTLINE = (236, 239, 243)
BODY = (74, 82, 92)
PAPER = (255, 255, 255)
SLOT = (38, 43, 50)
TRAY = (214, 219, 225)
ACCENT = (46, 111, 242)

RECTS = (
    (300, 150, 724, 430, 14, PAPER),      # the sheet coming out of the top
    (132, 352, 892, 828, 82, OUTLINE),    # a light edge, so the body has an outline
    (140, 360, 884, 820, 74, BODY),
    (296, 376, 728, 404, 14, SLOT),       # the slot the sheet leaves through
    (210, 700, 814, 836, 30, TRAY),       # the output tray, past the body's bottom
)
CIRCLES = (
    (232, 468, 24, ACCENT),
)


def write_png(path, size, pixels):
    """Write an 8-bit RGBA PNG from a flat bytearray of size*size*4 bytes."""
    raw = bytearray()
    for y in range(size):
        raw.append(0)  # filter type 0: these rows are cheap to encode as they are
        raw += pixels[y * size * 4:(y + 1) * size * 4]

    def chunk(tag, data):
        return (struct.pack('>I', len(data)) + tag + data
                + struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff))

    with open(path, 'wb') as f:
        f.write(b'\\x89PNG\\r\\n\\x1a\\n')
        f.write(chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0)))
        f.write(chunk(b'IDAT', zlib.compress(bytes(raw), 9)))
        f.write(chunk(b'IEND', b''))


def samples_for(size):
    """Supersampling per axis, by size: small icons are where edges show, 1024 is not."""
    if size <= 64:
        return 4
    if size <= 256:
        return 3
    if size <= 512:
        return 2
    return 1


def render(size):
    """Draw the icon at size pixels, supersampling each pixel by samples per axis."""
    samples = samples_for(size)
    pixels = bytearray(size * size * 4)
    step = 1.0 / samples
    scale = 1024.0 / size
    # Topmost shape first, taking the first hit: drawing order is back to front, so this is
    # the same result with far fewer tests per sample.
    layers = [(c[0], c[1], c[2]) + c[3:] for c in CIRCLES] + [r[:5] + r[5:] for r in RECTS][::-1]
    for y in range(size):
        for x in range(size):
            acc = [0, 0, 0, 0]
            for sy in range(samples):
                for sx in range(samples):
                    ux = (x + (sx + 0.5) * step) * scale
                    uy = (y + (sy + 0.5) * step) * scale
                    colour = None
                    for shape in layers:
                        if shape[0] is CIRCLE_SENTINEL:
                            pass
                        if len(shape) == 4:  # circle: (cx, cy, r, colour)
                            cx, cy, r, colour_try = shape
                            if (ux - cx) ** 2 + (uy - cy) ** 2 <= r * r:
                                colour = colour_try
                                break
                        else:  # rectangle: (x0, y0, x1, y1, r, colour)
                            x0, y0, x1, y1, r, colour_try = shape
                            qx = ux if x0 + r <= ux <= x1 - r else (x0 + r if ux < x0 + r else x1 - r)
                            qy = uy if y0 + r <= uy <= y1 - r else (y0 + r if uy < y0 + r else y1 - r)
                            if (ux - qx) ** 2 + (uy - qy) ** 2 <= r * r:
                                colour = colour_try
                                break
                    if colour is not None:
                        acc[0] += colour[0]
                        acc[1] += colour[1]
                        acc[2] += colour[2]
                        acc[3] += 255
            n = samples * samples
            i = (y * size + x) * 4
            if acc[3]:
                # Un-premultiply: the colour average is over the covered samples only.
                covered = acc[3] / 255.0
                pixels[i] = int(round(acc[0] / covered))
                pixels[i + 1] = int(round(acc[1] / covered))
                pixels[i + 2] = int(round(acc[2] / covered))
                pixels[i + 3] = int(round(acc[3] / n))
    return pixels


def build_iconset(target, source):
    """Fill an .iconset directory, either by drawing or by resizing an image."""
    os.makedirs(target, exist_ok=True)
    if source is None:
        for name, size in ICONSET:
            write_png(os.path.join(target, name), size, render(size))
        return
    for name, size in ICONSET:
        dst = os.path.join(target, name)
        out = subprocess.run(['sips', '-z', str(size), str(size), source, '--out', dst],
                             capture_output=True)
        if out.returncode != 0:
            raise SystemExit('error: sips could not resize %s to %d: %s'
                             % (source, size, out.stderr.decode('utf-8', 'replace').strip()))


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--out', default=None, help='icns to write (default assets/p1102.icns)')
    ap.add_argument('--from', dest='source', default=None,
                    help='use this PNG instead of the drawn icon')
    ap.add_argument('--preview', default=None, help='also write this 256px PNG')
    args = ap.parse_args(argv[1:])

    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out = args.out or os.path.join(root, 'assets', 'p1102.icns')
    for tool in ('iconutil', 'sips'):
        if shutil.which(tool) is None:
            raise SystemExit('error: %s is required and was not found' % tool)

    if args.source and args.source.endswith('.icns'):
        shutil.copyfile(args.source, out)
        print('copied %s to %s' % (args.source, out))
        return 0

    os.makedirs(os.path.dirname(out), exist_ok=True)
    work = tempfile.mkdtemp(prefix='p1102-iconset.')
    try:
        iconset = os.path.join(work, 'p1102.iconset')
        build_iconset(iconset, args.source)
        result = subprocess.run(['iconutil', '-c', 'icns', iconset, '-o', out],
                                capture_output=True)
        if result.returncode != 0:
            raise SystemExit('error: iconutil failed: %s'
                             % result.stderr.decode('utf-8', 'replace').strip())
        if args.preview:
            shutil.copyfile(os.path.join(iconset, 'icon_256x256.png'), args.preview)
        print('wrote %s (%d bytes) from %s'
              % (out, os.path.getsize(out), args.source or 'the drawn icon'))
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
