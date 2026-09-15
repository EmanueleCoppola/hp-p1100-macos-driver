#!/usr/bin/env python3
"""Build the .icns that our PPD points *APPrinterIconPath at.

macOS shows the PPD's icon next to the queue name - in System Settings, in the print
dialog, and anywhere else the print system reports a printer. That is measured here rather
than assumed: the row for HP's own queue matches /Library/Printers/hp/Icons/hp1100.icns to a
mean absolute luminance error of 8/255 across the icon box, while the stock system pictures
land between 44 and 98 away. The row is the icns file, not a generic glyph.

The picture comes from assets/p1102-photo.webp, a photograph of this printer, and the badge
on its front panel is repainted: HP's mark is removed and "EC" is drawn in the same dark
oval, in the same two colours the photograph uses. Everything else in the picture is left
alone, including the silver medallion around the oval and the model text beside it.

Two other pictures are one flag away: --drawn draws the icon in code instead (no
photograph, so it is ours outright and scales to any size), and --from takes any PNG.

Usage:
    python3 scripts/make-icns.py                        # from the photograph, badge relabelled
    python3 scripts/make-icns.py --drawn                # the drawn icon, no photograph
    python3 scripts/make-icns.py --from art.png         # any picture, no relabelling
    python3 scripts/make-icns.py --preview /tmp/i.png   # also write a 256px preview

Needs sips and iconutil, both stock macOS. sips also decodes the webp, because nothing
small in Python does.
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

# --- the drawn icon, used by --drawn -----------------------------------------
# The drawing, in a 1024-unit square, back to front. Rectangle entries are
# (x0, y0, x1, y1, corner radius, colour) with y measured downwards; the circle is
# (cx, cy, radius, colour). The last shape is the panel light, which is ours alone, and the
# mid-grey body with a light edge reads on a light and on a dark background alike.
OUTLINE = (236, 239, 243)
BODY = (112, 120, 132)
PAPER = (255, 255, 255)
EDGE = (150, 158, 170)
SLOT = (38, 43, 50)
TRAY = (214, 219, 225)
TRAY_EDGE = (166, 173, 184)
ACCENT = (46, 111, 242)

RECTS = (
    (318, 108, 706, 432, 20, EDGE),       # the sheet's edge, which shows on a white page
    (340, 130, 684, 410, 10, PAPER),      # the sheet standing out of the top slot
    (126, 326, 898, 812, 74, OUTLINE),    # a light edge, so the body has an outline
    (140, 340, 884, 800, 60, BODY),
    (318, 358, 706, 392, 16, SLOT),       # the slot the sheet leaves through
    (204, 774, 820, 906, 34, TRAY_EDGE),
    (214, 784, 810, 896, 28, TRAY),       # the output tray, out past the body's bottom
)
CIRCLES = (
    (250, 470, 22, ACCENT),
)

# --- the badge on the photograph, used by the default mode -------------------
# Measured on assets/p1102-photo.webp rather than guessed: the dark oval that carries the
# mark is 13x14 pixels at x 234..246, y 160..173, and its two colours are the median of its
# own dark pixels and of its own light ones. So the repaint keeps the photograph's palette
# and its soft edges, and only the letters change.
BADGE_CENTRE = (240.0, 166.5)
BADGE_RADII = (6.6, 7.2)
# The photograph's own badge is 13x14 pixels, which is 3% of the picture: at the sizes a
# printer row actually draws, its letters are one or two pixels and read as a smudge. So the
# oval is drawn at a chosen multiple of that instead, and the default is the smallest one
# whose letters still read (see the table in docs/installation.md). 1.0 is the photograph's
# own size, for when the icon is looked at rather than used.
BADGE_SCALE = 2.2
BADGE_LETTERS = (0.86, 0.72)      # fraction of the oval the two letters occupy
BADGE_STROKE = 0.14               # stroke width, as a fraction of letter height
BADGE_INSET = 5                   # pixels around the oval that get the same treatment
BADGE_SAMPLES = 12                # supersampling per axis while drawing the letters


def write_png(path, size, pixels):
    """Write an 8-bit RGBA PNG. pixels is a flat bytearray of size*size*4 bytes."""
    raw = bytearray()
    for y in range(size):
        raw.append(0)  # filter type 0: these rows are cheap to encode as they are
        raw += pixels[y * size * 4:(y + 1) * size * 4]

    def chunk(tag, data):
        return (struct.pack('>I', len(data)) + tag + data
                + struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff))

    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n')
        f.write(chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0)))
        f.write(chunk(b'IDAT', zlib.compress(bytes(raw), 9)))
        f.write(chunk(b'IEND', b''))


def read_png(path):
    """Read an 8-bit PNG into (width, height, rows of (r, g, b, a)).

    Deliberately small: it handles what sips writes, and refuses anything else rather than
    guessing. The alternative is a dependency, and this file has none.
    """
    data = open(path, 'rb').read()
    if data[:8] != b'\x89PNG\r\n\x1a\n':
        raise SystemExit('error: %s is not a PNG' % path)
    pos, idat, plte, trns = 8, b'', None, None
    width = height = depth = colour = None
    while pos + 8 <= len(data):
        length = struct.unpack('>I', data[pos:pos + 4])[0]
        tag = data[pos + 4:pos + 8]
        body = data[pos + 8:pos + 8 + length]
        if tag == b'IHDR':
            width, height, depth, colour, _, _, interlace = struct.unpack('>IIBBBBB', body)
            if depth != 8 or interlace != 0:
                raise SystemExit('error: %s is %d-bit or interlaced; only 8-bit plain PNG'
                                 % (path, depth))
        elif tag == b'IDAT':
            idat += body
        elif tag == b'PLTE':
            plte = body
        elif tag == b'tRNS':
            trns = body
        elif tag == b'IEND':
            break
        pos += 12 + length
    channels = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[colour]
    stride = width * channels
    raw = zlib.decompress(idat)
    rows = []
    previous = bytearray(stride)
    at = 0
    for _ in range(height):
        filter_type = raw[at]
        at += 1
        line = bytearray(raw[at:at + stride])
        at += stride
        if filter_type == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 255
        elif filter_type == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 255
        elif filter_type == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 255
        elif filter_type == 4:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                up = previous[i]
                upleft = previous[i - channels] if i >= channels else 0
                guess = left + up - upleft
                pa, pb, pc = abs(guess - left), abs(guess - up), abs(guess - upleft)
                best = left if (pa <= pb and pa <= pc) else (up if pb <= pc else upleft)
                line[i] = (line[i] + best) & 255
        elif filter_type != 0:
            raise SystemExit('error: %s uses PNG filter %d' % (path, filter_type))
        row = []
        for x in range(width):
            i = x * channels
            if colour == 6:
                row.append(tuple(line[i:i + 4]))
            elif colour == 2:
                row.append((line[i], line[i + 1], line[i + 2], 255))
            elif colour == 0:
                row.append((line[i], line[i], line[i], 255))
            elif colour == 4:
                row.append((line[i], line[i], line[i], line[i + 1]))
            else:
                index = line[i] * 3
                alpha = trns[line[i]] if trns else 255
                row.append((plte[index], plte[index + 1], plte[index + 2], alpha))
        rows.append(row)
        previous = line
    return width, height, rows


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
    # Topmost shape first, taking the first hit. The drawing is defined back to front, so
    # this is the same picture with far fewer tests per sample.
    layers = [("c",) + c for c in CIRCLES] + [("r",) + r for r in RECTS][::-1]
    for y in range(size):
        for x in range(size):
            acc = [0, 0, 0, 0]
            for sy in range(samples):
                for sx in range(samples):
                    ux = (x + (sx + 0.5) * step) * scale
                    uy = (y + (sy + 0.5) * step) * scale
                    colour = None
                    for shape in layers:
                        if shape[0] == "c":
                            _, cx, cy, r, c = shape
                            if (ux - cx) ** 2 + (uy - cy) ** 2 <= r * r:
                                colour = c
                                break
                        else:
                            _, x0, y0, x1, y1, r, c = shape
                            qx = min(max(ux, x0 + r), x1 - r)
                            qy = min(max(uy, y0 + r), y1 - r)
                            if (ux - qx) ** 2 + (uy - qy) ** 2 <= r * r:
                                colour = c
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


# --- relabelling the badge ---------------------------------------------------

def luminance(pixel):
    return 0.299 * pixel[0] + 0.587 * pixel[1] + 0.114 * pixel[2]


def median_colour(pixels, channel):
    return sorted(p[channel] for p in pixels)[len(pixels) // 2]


def letter_shape(u, v):
    """True inside the mark, for u and v over the box holding both letters.

    The two letters are geometrically simple on purpose: at 13x14 source pixels there is no
    room for a typeface, and a shape that stays readable at this size is the whole point.
    """
    width, height = BADGE_LETTERS
    x0, y0 = (1 - width) / 2, (1 - height) / 2
    if not (x0 <= u <= x0 + width and y0 <= v <= y0 + height):
        return False
    lu, lv = (u - x0) / width, (v - y0) / height
    stroke = BADGE_STROKE
    if lu < 0.5:                                   # E
        uu = lu / 0.46
        if uu > 1:
            return False
        if uu <= stroke:
            return True
        if lv <= stroke and uu <= 0.86:
            return True
        if 0.43 <= lv <= 0.57 and uu <= 0.68:
            return True
        return lv >= 1 - stroke and uu <= 0.86
    uu = (lu - 0.54) / 0.46                        # C
    if uu < 0 or uu > 1:
        return False
    du, dv = (uu - 0.5) * 2, (lv - 0.5) * 2
    radius = du * du + dv * dv
    if radius > 1.0:
        return False
    inner = 1.0 - stroke * 2.6
    if radius < inner * inner:
        return False
    return not (du > 0.25 and abs(dv) < 0.30)      # the opening on the right


def relabel_badge(width, height, rows, scale=BADGE_SCALE):
    """Replace the badge's mark with EC, keeping its oval, its colours and its softness."""
    centre = BADGE_CENTRE
    radii = (BADGE_RADII[0] * scale, BADGE_RADII[1] * scale)
    inside = [(x, y)
              for y in range(int(centre[1] - BADGE_RADII[1]) - BADGE_INSET,
                             int(centre[1] + BADGE_RADII[1]) + BADGE_INSET + 1)
              for x in range(int(centre[0] - BADGE_RADII[0]) - BADGE_INSET,
                             int(centre[0] + BADGE_RADII[0]) + BADGE_INSET + 1)
              if 0 <= x < width and 0 <= y < height and rows[y][x][3] > 128]
    if not inside:
        raise SystemExit('error: the badge is not where the photograph said it was')
    dark = [rows[y][x] for x, y in inside if luminance(rows[y][x]) < 90]
    light = [rows[y][x] for x, y in inside if luminance(rows[y][x]) > 190]
    if not dark or not light:
        raise SystemExit('error: the badge has no distinct dark and light pixels')
    core = tuple(median_colour(dark, c) for c in range(3))
    ink = tuple(median_colour(light, c) for c in range(3))

    cx, cy = centre
    rx, ry = radii
    out = [row[:] for row in rows]
    if scale > 1.0:
        # An enlarged oval has to cover the photograph's own badge, and blend into the panel
        # around it. The panel's colour is sampled just outside the new oval, per row, so the
        # gradient across the front door survives the patch.
        fade = 1.6
        for y in range(int(cy - ry * (1 + fade)) - 1, int(cy + ry * (1 + fade)) + 2):
            if not (0 <= y < height):
                continue
            sample = rows[y][min(width - 1, max(0, int(cx + rx * 1.5)))]
            panel = sample[:3]
            for x in range(int(cx - rx * (1 + fade)) - 1, int(cx + rx * (1 + fade)) + 2):
                if not (0 <= x < width):
                    continue
                base = rows[y][x]
                if base[3] < 128:
                    continue
                dx, dy = (x + 0.5 - cx) / rx, (y + 0.5 - cy) / ry
                radius = (dx * dx + dy * dy) ** 0.5
                if radius > 1.0 + fade:
                    continue
                t = 1.0 if radius <= 1.0 else max(0.0, (1.0 + fade - radius) / fade)
                out[y][x] = (int(base[0] * (1 - t) + panel[0] * t),
                             int(base[1] * (1 - t) + panel[1] * t),
                             int(base[2] * (1 - t) + panel[2] * t), base[3])
    for y in range(int(cy - ry) - BADGE_INSET, int(cy + ry) + BADGE_INSET + 1):
        for x in range(int(cx - rx) - BADGE_INSET, int(cx + rx) + BADGE_INSET + 1):
            if not (0 <= x < width and 0 <= y < height):
                continue
            base = rows[y][x]
            if base[3] < 128:
                continue
            colour = [0, 0, 0]
            covered = 0
            for sy in range(BADGE_SAMPLES):
                for sx in range(BADGE_SAMPLES):
                    ux = x + (sx + 0.5) / BADGE_SAMPLES
                    uy = y + (sy + 0.5) / BADGE_SAMPLES
                    dx, dy = (ux - cx) / rx, (uy - cy) / ry
                    if dx * dx + dy * dy > 1.0:
                        continue
                    covered += 1
                    paint = ink if letter_shape((ux - (cx - rx)) / (2 * rx),
                                                (uy - (cy - ry)) / (2 * ry)) else core
                    colour[0] += paint[0]
                    colour[1] += paint[1]
                    colour[2] += paint[2]
            if not covered:
                continue
            share = covered / float(BADGE_SAMPLES * BADGE_SAMPLES)
            out[y][x] = (int(base[0] * (1 - share) + colour[0] / covered * share),
                         int(base[1] * (1 - share) + colour[1] / covered * share),
                         int(base[2] * (1 - share) + colour[2] / covered * share),
                         base[3])
    return out


# --- assembling an .iconset --------------------------------------------------

def square(width, height, rows, margin=0.04):
    """Centre a picture on a transparent square canvas, so no size distorts it."""
    side = int(round(max(width, height) * (1 + margin)))
    canvas = bytearray(side * side * 4)
    left, top = (side - width) // 2, (side - height) // 2
    for y in range(height):
        for x in range(width):
            r, g, b, a = rows[y][x]
            i = ((top + y) * side + left + x) * 4
            canvas[i:i + 4] = bytes((r, g, b, a))
    return side, canvas


def decode(path, work):
    """Read any picture sips can read, as (width, height, rows)."""
    if path.lower().endswith('.png'):
        return read_png(path)
    png = os.path.join(work, 'source.png')
    result = subprocess.run(['sips', '-s', 'format', 'png', path, '--out', png],
                            capture_output=True)
    if result.returncode != 0 or not os.path.exists(png):
        raise SystemExit('error: sips could not read %s: %s'
                         % (path, result.stderr.decode('utf-8', 'replace').strip()))
    return read_png(png)


def main(argv):
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    default_photo = os.path.join(root, 'assets', 'p1102-photo.webp')

    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--out', default=os.path.join(root, 'assets', 'p1100.icns'))
    ap.add_argument('--photo', default=default_photo,
                    help='photograph to relabel and use (default assets/p1102-photo.webp)')
    ap.add_argument('--drawn', action='store_true', help='draw the icon instead')
    ap.add_argument('--from', dest='source', default=None,
                    help='use this picture as it is, without relabelling')
    ap.add_argument('--badge-scale', type=float, default=BADGE_SCALE,
                    help="badge size as a multiple of the photograph's own (default %.1f)"
                         % BADGE_SCALE)
    ap.add_argument('--preview', default=None, help='also write this 256px PNG')
    args = ap.parse_args(argv[1:])

    for tool in ('sips', 'iconutil'):
        if shutil.which(tool) is None:
            raise SystemExit('error: %s is required and was not found' % tool)

    work = tempfile.mkdtemp(prefix='p1100-iconset.')
    try:
        if args.drawn:
            how = 'the drawn icon'
        else:
            picture = args.source or args.photo
            if not os.path.exists(picture):
                raise SystemExit('error: %s is missing; pass --photo FILE, --from FILE or '
                                 '--drawn' % picture)
            width, height, rows = decode(picture, work)
            if args.source is None:
                rows = relabel_badge(width, height, rows, args.badge_scale)
                how = ('%s with the badge relabelled at %.1fx'
                       % (picture, args.badge_scale))
            else:
                how = picture
            side, canvas = square(width, height, rows)
            square_png = os.path.join(work, 'square.png')
            write_png(square_png, side, canvas)

        iconset = os.path.join(work, 'p1100.iconset')
        os.makedirs(iconset)
        for name, size in ICONSET:
            target = os.path.join(iconset, name)
            if args.drawn:
                write_png(target, size, render(size))
            else:
                # A square canvas is what lets sips do the resampling: it keeps the
                # picture's proportions, where sips -z would squash them into a square.
                result = subprocess.run(['sips', '-z', str(size), str(size), square_png,
                                         '--out', target], capture_output=True)
                if result.returncode != 0:
                    raise SystemExit('error: sips could not resize to %d: %s'
                                     % (size, result.stderr.decode('utf-8', 'replace').strip()))
        if args.preview:
            shutil.copyfile(os.path.join(iconset, 'icon_256x256.png'), args.preview)
        os.makedirs(os.path.dirname(args.out), exist_ok=True)
        result = subprocess.run(['iconutil', '-c', 'icns', iconset, '-o', args.out],
                                capture_output=True)
        if result.returncode != 0:
            raise SystemExit('error: iconutil failed: %s'
                             % result.stderr.decode('utf-8', 'replace').strip())
        print('wrote %s (%d bytes) from %s'
              % (args.out, os.path.getsize(args.out), how))
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
