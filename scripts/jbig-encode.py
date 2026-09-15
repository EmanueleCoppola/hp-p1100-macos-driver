#!/usr/bin/env python3
"""JBIG (ISO/IEC 11544 / ITU-T T.82) encoder, written from the standard.

Configuration emitted, the one both working drivers use for this printer family:

    DL = 0, D = 0 (single resolution layer), planes = 1
    order   = 3       (ILEAVE | SMID; irrelevant with one plane and one layer)
    options = 0x5c    (DPON | TPBON | TPDON | LRLTWO)
    L0      = 128     lines per stripe
    MX = MY = 0       no adaptive template movement

Only TPBON changes the coded data here: TPDON and LRLTWO belong to the low-resolution
image of differential layers, which we do not emit, and DPON is a decoder-side switch.
LRLTWO still selects which of the two typical-prediction contexts is used.

The arithmetic coder is the standard's QM coder and the probability table is the
normative one published in T.82 - it is a table of constants every implementation has to
contain, not copied implementation code.

This is the prototype the Rust implementation in crates/p1102-jbig has to match, and it
is checked byte for byte against the reference encoder:

    scripts/jbig-oracle.sh encode x.pbm ref.jbg        # -p 92 is the script's default
    scripts/jbig-encode.py x.pbm ours.jbg
    cmp ref.jbg ours.jbg

Usage:
  scripts/jbig-encode.py <in.pbm> <out.jbg> [--l0 N] [--options N] [--quiet]
"""

import argparse
import sys

# BIH option bits. DPON, TPBON, TPDON and LRLTWO together are what HP's driver stores in
# its encoder constructor and what the reference implementation sends for this family.
OPT_DPON = 0x04
OPT_TPBON = 0x08
OPT_TPDON = 0x10
OPT_LRLTWO = 0x40
DEFAULT_OPTIONS = OPT_DPON | OPT_TPBON | OPT_TPDON | OPT_LRLTWO

# The two contexts T.82 reserves for the typical-prediction bit, selected by LRLTWO.
TP_CX_LRLTWO = 0x195
TP_CX_DEFAULT = 0x0E5

# --------------------------------------------------------------------------- tables

# Normative T.82 QM-coder tables, indexed by probability state 0..112.
LSZ = [
    0x5A1D, 0x2586, 0x1114, 0x080B, 0x03D8, 0x01DA, 0x00E5, 0x006F,
    0x0036, 0x001A, 0x000D, 0x0006, 0x0003, 0x0001, 0x5A7F, 0x3F25,
    0x2CF2, 0x207C, 0x17B9, 0x1182, 0x0CEF, 0x09A1, 0x072F, 0x055C,
    0x0406, 0x0303, 0x0240, 0x01B1, 0x0144, 0x00F5, 0x00B7, 0x008A,
    0x0068, 0x004E, 0x003B, 0x002C, 0x5AE1, 0x484C, 0x3A0D, 0x2EF1,
    0x261F, 0x1F33, 0x19A8, 0x1518, 0x1177, 0x0E74, 0x0BFB, 0x09F8,
    0x0861, 0x0706, 0x05CD, 0x04DE, 0x040F, 0x0363, 0x02D4, 0x025C,
    0x01F8, 0x01A4, 0x0160, 0x0125, 0x00F6, 0x00CB, 0x00AB, 0x008F,
    0x5B12, 0x4D04, 0x412C, 0x37D8, 0x2FE8, 0x293C, 0x2379, 0x1EDF,
    0x1AA9, 0x174E, 0x1424, 0x119C, 0x0F6B, 0x0D51, 0x0BB6, 0x0A40,
    0x5832, 0x4D1C, 0x438E, 0x3BDD, 0x34EE, 0x2EAE, 0x299A, 0x2516,
    0x5570, 0x4CA9, 0x44D9, 0x3E22, 0x3824, 0x32B4, 0x2E17, 0x56A8,
    0x4F46, 0x47E5, 0x41CF, 0x3C3D, 0x375E, 0x5231, 0x4C0F, 0x4639,
    0x415E, 0x5627, 0x50E7, 0x4B85, 0x5597, 0x504F, 0x5A10, 0x5522,
    0x59EB,
]

NMPS = [
    1, 2, 3, 4, 5, 6, 7, 8,
    9, 10, 11, 12, 13, 13, 15, 16,
    17, 18, 19, 20, 21, 22, 23, 24,
    25, 26, 27, 28, 29, 30, 31, 32,
    33, 34, 35, 9, 37, 38, 39, 40,
    41, 42, 43, 44, 45, 46, 47, 48,
    49, 50, 51, 52, 53, 54, 55, 56,
    57, 58, 59, 60, 61, 62, 63, 32,
    65, 66, 67, 68, 69, 70, 71, 72,
    73, 74, 75, 76, 77, 78, 79, 48,
    81, 82, 83, 84, 85, 86, 87, 71,
    89, 90, 91, 92, 93, 94, 86, 96,
    97, 98, 99, 100, 93, 102, 103, 104,
    99, 106, 107, 103, 109, 107, 111, 109,
    111,
]

# Low 7 bits are the next state for the less probable symbol; bit 7 is SWTCH.
NLPS = [
    129, 14, 16, 18, 20, 23, 25, 28,
    30, 33, 35, 9, 10, 12, 143, 36,
    38, 39, 40, 42, 43, 45, 46, 48,
    49, 51, 52, 54, 56, 57, 59, 60,
    62, 63, 32, 33, 165, 64, 65, 67,
    68, 69, 70, 72, 73, 74, 75, 77,
    78, 79, 48, 50, 50, 51, 52, 53,
    54, 55, 56, 57, 58, 59, 61, 61,
    193, 80, 81, 82, 83, 84, 86, 87,
    87, 72, 72, 74, 74, 75, 77, 77,
    208, 88, 89, 90, 91, 92, 93, 86,
    216, 95, 96, 97, 99, 99, 93, 223,
    101, 102, 103, 104, 99, 105, 106, 107,
    103, 233, 108, 109, 110, 111, 238, 112,
    240,
]

MARKER_STUFF = 0x00
MARKER_ESC = 0xFF
MARKER_SDNORM = 0x02
MARKER_SDRST = 0x03


# --------------------------------------------------------------------------- QM coder

class QmEncoder(object):
    """The T.82 QM arithmetic coder, encoder side.

    The probability state table is shared across stripes: T.82 only resets it at the
    first stripe, unless SDRST is in use. The code/interval registers are reset for
    every stripe.
    """

    def __init__(self, st, reuse_state):
        if not reuse_state:
            for i in range(len(st)):
                st[i] = 0
        self.st = st
        self.c = 0
        self.a = 0x10000
        self.sc = 0          # count of buffered 0xFF bytes
        self.ct = 11         # bits until the next byte is ready
        self.buffer = -1     # empty
        self.out = bytearray()

    def _byte_out(self, b):
        self.out.append(b & 0xFF)

    def encode(self, cx, pix):
        st = self.st
        state = st[cx]
        ss = state & 0x7F
        lsz = LSZ[ss]

        if ((pix << 7) ^ state) & 0x80:
            # less probable symbol
            self.a -= lsz
            if self.a >= lsz:
                self.c += self.a
                self.a = lsz
            st[cx] = (state & 0x80) ^ NLPS[ss]
        else:
            # more probable symbol
            self.a -= lsz
            if self.a & 0xFFFF8000:
                return
            if self.a < lsz:
                self.c += self.a
                self.a = lsz
            st[cx] = (state & 0x80) | NMPS[ss]

        while True:
            self.a <<= 1
            self.c <<= 1
            self.ct -= 1
            if self.ct == 0:
                temp = self.c >> 19
                if temp & 0xFFFFFF00:
                    if self.buffer >= 0:
                        self._byte_out(self.buffer + 1)
                        if self.buffer + 1 == MARKER_ESC:
                            self._byte_out(MARKER_STUFF)
                    while self.sc:
                        self._byte_out(0x00)
                        self.sc -= 1
                    self.buffer = temp & 0xFF
                elif temp == 0xFF:
                    self.sc += 1
                else:
                    if self.buffer >= 0:
                        self._byte_out(self.buffer)
                    while self.sc:
                        self._byte_out(MARKER_ESC)
                        self._byte_out(MARKER_STUFF)
                        self.sc -= 1
                    self.buffer = temp
                self.c &= 0x7FFFF
                self.ct = 8
            if not self.a < 0x8000:
                break

    def flush(self):
        temp = (self.a - 1 + self.c) & 0xFFFF0000
        self.c = temp + 0x8000 if temp < self.c else temp
        self.c <<= self.ct
        if self.c & 0xF8000000:
            if self.buffer >= 0:
                self._byte_out(self.buffer + 1)
                if self.buffer + 1 == MARKER_ESC:
                    self._byte_out(MARKER_STUFF)
            # 0x00 bytes are only emitted when more non-0x00 will follow
            if self.c & 0x7FFF800:
                while self.sc:
                    self._byte_out(0x00)
                    self.sc -= 1
        else:
            if self.buffer >= 0:
                self._byte_out(self.buffer)
            while self.sc:
                self._byte_out(MARKER_ESC)
                self._byte_out(MARKER_STUFF)
                self.sc -= 1
        if self.c & 0x7FFF800:
            self._byte_out((self.c >> 19) & 0xFF)
            if ((self.c >> 19) & 0xFF) == MARKER_ESC:
                self._byte_out(MARKER_STUFF)
            if self.c & 0x7F800:
                self._byte_out((self.c >> 11) & 0xFF)
                if ((self.c >> 11) & 0xFF) == MARKER_ESC:
                    self._byte_out(MARKER_STUFF)


# --------------------------------------------------------------------------- JBIG

class JbigEncoder(object):
    def __init__(self, width, height, l0=128, order=3, options=DEFAULT_OPTIONS):
        self.xd = width
        self.yd = height
        self.l0 = l0
        self.order = order
        self.options = options
        self.hbpl = (width + 7) // 8          # bytes per line, padding included
        self.stripes = (height + l0 - 1) // l0 if height else 0
        self.st = bytearray(4096)

    def bih(self):
        b = bytearray(20)
        b[0] = 0                    # DL
        b[1] = 0                    # D, highest layer
        b[2] = 1                    # one bit plane
        b[3] = 0
        b[4:8] = self.xd.to_bytes(4, 'big')
        b[8:12] = self.yd.to_bytes(4, 'big')
        b[12:16] = self.l0.to_bytes(4, 'big')
        b[16] = 0                   # MX
        b[17] = 0                   # MY
        b[18] = self.order
        b[19] = self.options & 0x7F
        return bytes(b)

    def encode(self, bitmap):
        if len(bitmap) < self.hbpl * self.yd:
            raise ValueError('bitmap too small: %d bytes, need %d'
                             % (len(bitmap), self.hbpl * self.yd))
        out = bytearray(self.bih())
        for stripe in range(self.stripes):
            out += self._encode_sde(bitmap, stripe)
        return bytes(out)

    def _encode_sde(self, bitmap, stripe):
        hbpl, xd, yd = self.hbpl, self.xd, self.yd
        hl = self.l0
        y0 = stripe * hl
        reset = (stripe == 0)

        enc = QmEncoder(self.st, reuse_state=not reset)
        lines = min(hl, yd - y0)

        # Typical prediction: one bit per line saying whether the line is still typical
        # (identical to the line above, or blank for the very first line), and a typical
        # line is then not coded at all. LRLTWO picks which of the two reserved contexts
        # the bit is coded with.
        tp = bool(self.options & OPT_TPBON)
        lrltwo = bool(self.options & OPT_LRLTWO)
        tp_cx = TP_CX_LRLTWO if lrltwo else TP_CX_DEFAULT
        if reset:
            ltp_old = False
        else:
            prev = (y0 - 1) * hbpl
            if y0 > 1:
                above = (y0 - 2) * hbpl
                ltp_old = bitmap[prev:prev + hbpl] == bitmap[above:above + hbpl]
            else:
                ltp_old = not any(bitmap[prev:prev + hbpl])

        for i in range(lines):
            y = y0 + i
            base = y * hbpl
            up = base - hbpl
            up2 = base - hbpl - hbpl

            if tp:
                if i > 0 or not reset:
                    typical = bitmap[base:base + hbpl] == bitmap[up:base]
                else:
                    typical = not any(bitmap[base:base + hbpl])
                enc.encode(tp_cx, int(typical == ltp_old))
                ltp_old = typical
                if typical:
                    continue

            line_h1 = 0
            line_h2 = 0
            line_h3 = 0
            if i > 0 or not reset:
                line_h2 = bitmap[up] << 8
            if i > 1 or not reset:
                line_h3 = bitmap[up2] << 8

            j = 0
            k = 0
            while j < xd:
                line_h1 |= bitmap[base + k]
                if j < hbpl * 8 - 8 and (i > 0 or not reset):
                    line_h2 |= bitmap[up + k + 1]
                    if i > 1 or not reset:
                        line_h3 |= bitmap[up2 + k + 1]

                while True:
                    line_h1 <<= 1
                    line_h2 <<= 1
                    line_h3 <<= 1
                    # The layer being coded is the lowest resolution layer - with no
                    # differential layers it is also the only one - and LRLTWO picks its
                    # template. Neither has an adaptive-template pixel here.
                    if lrltwo:
                        # Two-line template:
                        #   row y-1: columns x-4..x+1 -> line_h2 bits 14..19
                        #   row y  : columns x-4..x-1 -> line_h1 bits  9..12
                        cx = (((line_h2 >> 10) & 0x3F0)
                              | ((line_h1 >> 9) & 0x00F))
                    else:
                        # Three-line template:
                        #   row y-2: columns x+1..x+3 -> line_h3 bits 11..13
                        #   row y-1: columns x-4..x   -> line_h2 bits 14..18
                        #   row y  : columns x-2..x-1 -> line_h1 bits  9..10
                        cx = (((line_h3 >> 8) & 0x380)
                              | ((line_h2 >> 12) & 0x07C)
                              | ((line_h1 >> 9) & 0x003))
                    enc.encode(cx, (line_h1 >> 8) & 1)
                    j += 1
                    if not (j & 7 and j < xd):
                        break
                k += 1

        enc.flush()
        body = bytearray(enc.out)
        # Trailing zero bytes are not part of the stripe data.
        while body and body[-1] == 0:
            body.pop()
        # Keep the 0xFF escaping unambiguous across the terminator.
        if body and body[-1] == MARKER_ESC:
            body.append(MARKER_STUFF)
        body.append(MARKER_ESC)
        body.append(MARKER_SDRST if (self.options & 0x200) else MARKER_SDNORM)
        return bytes(body)


# --------------------------------------------------------------------------- PBM input

def read_pbm(path):
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
    i += 1
    w, h = vals
    return w, h, d[i:]


def main(argv):
    ap = argparse.ArgumentParser(add_help=True)
    ap.add_argument('input')
    ap.add_argument('output')
    ap.add_argument('--l0', type=int, default=128)
    ap.add_argument('--options', type=lambda s: int(s, 0), default=DEFAULT_OPTIONS,
                    help='BIH options byte (default 0x5c, the reference configuration)')
    ap.add_argument('--quiet', action='store_true')
    args = ap.parse_args(argv)

    w, h, data = read_pbm(args.input)
    enc = JbigEncoder(w, h, l0=args.l0, options=args.options)
    bie = enc.encode(data)
    with open(args.output, 'wb') as f:
        f.write(bie)
    if not args.quiet:
        print('%s: %dx%d -> %d bytes (%d stripes, L0=%d, options=0x%02x)'
              % (args.output, w, h, len(bie), enc.stripes, args.l0, args.options))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
