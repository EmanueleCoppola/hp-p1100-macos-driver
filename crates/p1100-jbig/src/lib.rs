//! JBIG (ISO/IEC 11544 / ITU-T T.82) encoder.
//!
//! Written from the standard. Configuration emitted:
//!
//! ```text
//! DL = 0, D = 0 (a single resolution layer), planes = 1
//! order   = 3    (ILEAVE | SMID; irrelevant with one plane and one layer)
//! options = 0x5c (DPON | TPBON | TPDON | LRLTWO)
//! L0      = 128  lines per stripe
//! MX = MY = 0    no adaptive template movement
//! ```
//!
//! This is the configuration both working drivers use for this printer family, and it is
//! not an arbitrary choice: HP's own filter stores `0x5c` in the encoder constructor, and
//! the reference implementation reaches the same byte. The product is byte-identical to
//! the reference implementation for this configuration, which the oracle sweep and the
//! golden tests below check; see `docs/jbig.md`.
//!
//! Of the four bits, only TPBON changes the coded data here. TPDON and LRLTWO apply to the
//! low-resolution image of differential layers, which we do not emit, and DPON is a
//! decoder-side switch. LRLTWO is not quite inert even so: it selects which of the two
//! contexts T.82 reserves for the typical-prediction bit is used.
//!
//! The probability table is the normative one from T.82: a table of constants that every
//! implementation has to contain, not copied implementation code.
//!
//! Scope is deliberately narrow: one or more bit planes, one resolution layer, no adaptive template
//! movement, no differential layers. The pages this driver sends need nothing more.

/// Lines per stripe. The value the reference implementation uses for this printer family.
pub const DEFAULT_L0: u32 = 128;
/// `ILEAVE | SMID`, matching the reference implementation's default order byte.
pub const DEFAULT_ORDER: u8 = 3;

/// BIH `options` bits.
pub const OPT_DPON: u8 = 0x04;
pub const OPT_TPBON: u8 = 0x08;
pub const OPT_TPDON: u8 = 0x10;
pub const OPT_LRLTWO: u8 = 0x40;

/// `DPON | TPBON | TPDON | LRLTWO`, the value HP's driver and the reference implementation
/// both put in the BIH for this printer family.
pub const DEFAULT_OPTIONS: u8 = OPT_DPON | OPT_TPBON | OPT_TPDON | OPT_LRLTWO;

/// The two arithmetic-coder contexts T.82 reserves for the typical-prediction bit, chosen
/// by LRLTWO.
const TP_CX_LRLTWO: usize = 0x195;
const TP_CX_DEFAULT: usize = 0x0E5;

const MARKER_STUFF: u8 = 0x00;
const MARKER_ESC: u8 = 0xFF;
const MARKER_SDNORM: u8 = 0x02;

/// Number of contexts the QM coder keeps probability estimates for.
const CONTEXTS: usize = 4096;

/// Normative T.82 QM-coder table: interval size for each probability state.
const LSZ: [u32; 113] = [
    0x5A1D, 0x2586, 0x1114, 0x080B, 0x03D8, 0x01DA, 0x00E5, 0x006F, 0x0036, 0x001A, 0x000D, 0x0006,
    0x0003, 0x0001, 0x5A7F, 0x3F25, 0x2CF2, 0x207C, 0x17B9, 0x1182, 0x0CEF, 0x09A1, 0x072F, 0x055C,
    0x0406, 0x0303, 0x0240, 0x01B1, 0x0144, 0x00F5, 0x00B7, 0x008A, 0x0068, 0x004E, 0x003B, 0x002C,
    0x5AE1, 0x484C, 0x3A0D, 0x2EF1, 0x261F, 0x1F33, 0x19A8, 0x1518, 0x1177, 0x0E74, 0x0BFB, 0x09F8,
    0x0861, 0x0706, 0x05CD, 0x04DE, 0x040F, 0x0363, 0x02D4, 0x025C, 0x01F8, 0x01A4, 0x0160, 0x0125,
    0x00F6, 0x00CB, 0x00AB, 0x008F, 0x5B12, 0x4D04, 0x412C, 0x37D8, 0x2FE8, 0x293C, 0x2379, 0x1EDF,
    0x1AA9, 0x174E, 0x1424, 0x119C, 0x0F6B, 0x0D51, 0x0BB6, 0x0A40, 0x5832, 0x4D1C, 0x438E, 0x3BDD,
    0x34EE, 0x2EAE, 0x299A, 0x2516, 0x5570, 0x4CA9, 0x44D9, 0x3E22, 0x3824, 0x32B4, 0x2E17, 0x56A8,
    0x4F46, 0x47E5, 0x41CF, 0x3C3D, 0x375E, 0x5231, 0x4C0F, 0x4639, 0x415E, 0x5627, 0x50E7, 0x4B85,
    0x5597, 0x504F, 0x5A10, 0x5522, 0x59EB,
];

/// Next state after coding the more probable symbol.
const NMPS: [u8; 113] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 13, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 33, 34, 35, 9, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50,
    51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 32, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74,
    75, 76, 77, 78, 79, 48, 81, 82, 83, 84, 85, 86, 87, 71, 89, 90, 91, 92, 93, 94, 86, 96, 97, 98,
    99, 100, 93, 102, 103, 104, 99, 106, 107, 103, 109, 107, 111, 109, 111,
];

/// Next state after coding the less probable symbol, with the SWTCH flag in bit 7.
const NLPS: [u8; 113] = [
    129, 14, 16, 18, 20, 23, 25, 28, 30, 33, 35, 9, 10, 12, 143, 36, 38, 39, 40, 42, 43, 45, 46,
    48, 49, 51, 52, 54, 56, 57, 59, 60, 62, 63, 32, 33, 165, 64, 65, 67, 68, 69, 70, 72, 73, 74,
    75, 77, 78, 79, 48, 50, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 61, 61, 193, 80, 81, 82, 83,
    84, 86, 87, 87, 72, 72, 74, 74, 75, 77, 77, 208, 88, 89, 90, 91, 92, 93, 86, 216, 95, 96, 97,
    99, 99, 93, 223, 101, 102, 103, 104, 99, 105, 106, 107, 103, 233, 108, 109, 110, 111, 238, 112,
    240,
];

/// Bits tested by the flush path to decide whether trailing zero bytes are significant.
const FLUSH_SIG: u64 = 0x07FF_F800;
const FLUSH_SIG2: u64 = 0x0007_F800;

/// The T.82 QM arithmetic coder, encoder side.
///
/// The probability state table is owned by the caller so it can be shared across stripes:
/// T.82 resets it only at the first stripe unless SDRST is in use. The code and interval
/// registers are reset for every stripe.
pub struct QmEncoder<'a> {
    st: &'a mut [u8],
    c: u64,
    a: u32,
    sc: u32,
    ct: u32,
    buffer: i32,
    out: Vec<u8>,
}

impl<'a> QmEncoder<'a> {
    pub fn new(st: &'a mut [u8], reuse_state: bool) -> Self {
        if !reuse_state {
            st.fill(0);
        }
        Self {
            st,
            c: 0,
            a: 0x1_0000,
            sc: 0,
            ct: 11,
            buffer: -1,
            out: Vec::new(),
        }
    }

    fn byte_out(&mut self, b: u8) {
        self.out.push(b);
    }

    /// Code one pixel. `pix` must be 0 or 1.
    pub fn encode(&mut self, cx: usize, pix: u32) {
        debug_assert!(cx < CONTEXTS);
        let state = self.st[cx];
        let ss = (state & 0x7F) as usize;
        let lsz = LSZ[ss];

        if ((pix << 7) ^ u32::from(state)) & 0x80 != 0 {
            // less probable symbol
            self.a -= lsz;
            if self.a >= lsz {
                self.c += u64::from(self.a);
                self.a = lsz;
            }
            self.st[cx] = (state & 0x80) ^ NLPS[ss];
        } else {
            // more probable symbol
            self.a -= lsz;
            if self.a & 0xFFFF_8000 != 0 {
                return; // still >= 0x8000, no renormalisation needed
            }
            if self.a < lsz {
                self.c += u64::from(self.a);
                self.a = lsz;
            }
            self.st[cx] = (state & 0x80) | NMPS[ss];
        }

        loop {
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.ct == 0 {
                let temp = self.c >> 19;
                if temp & 0xFFFF_FF00 != 0 {
                    // carry out of all the buffered 0xFF bytes
                    if self.buffer >= 0 {
                        let b = (self.buffer + 1) as u8;
                        self.byte_out(b);
                        if b == MARKER_ESC {
                            self.byte_out(MARKER_STUFF);
                        }
                    }
                    while self.sc > 0 {
                        self.byte_out(0x00);
                        self.sc -= 1;
                    }
                    self.buffer = (temp & 0xFF) as i32;
                } else if temp == 0xFF {
                    self.sc += 1;
                } else {
                    if self.buffer >= 0 {
                        self.byte_out(self.buffer as u8);
                    }
                    while self.sc > 0 {
                        self.byte_out(MARKER_ESC);
                        self.byte_out(MARKER_STUFF);
                        self.sc -= 1;
                    }
                    self.buffer = temp as i32;
                }
                self.c &= 0x7_FFFF;
                self.ct = 8;
            }
            if self.a >= 0x8000 {
                break;
            }
        }
    }

    /// Terminate the stripe's coded data.
    pub fn flush(&mut self) {
        let temp = (u64::from(self.a) - 1 + self.c) & 0xFFFF_0000;
        self.c = if temp < self.c { temp + 0x8000 } else { temp };
        self.c <<= self.ct;
        if self.c & 0xF800_0000 != 0 {
            if self.buffer >= 0 {
                let b = (self.buffer + 1) as u8;
                self.byte_out(b);
                if b == MARKER_ESC {
                    self.byte_out(MARKER_STUFF);
                }
            }
            // zero bytes are emitted only when more non-zero data will follow
            if self.c & FLUSH_SIG != 0 {
                while self.sc > 0 {
                    self.byte_out(0x00);
                    self.sc -= 1;
                }
            }
        } else {
            if self.buffer >= 0 {
                self.byte_out(self.buffer as u8);
            }
            while self.sc > 0 {
                self.byte_out(MARKER_ESC);
                self.byte_out(MARKER_STUFF);
                self.sc -= 1;
            }
        }
        if self.c & FLUSH_SIG != 0 {
            self.byte_out(((self.c >> 19) & 0xFF) as u8);
            if ((self.c >> 19) & 0xFF) as u8 == MARKER_ESC {
                self.byte_out(MARKER_STUFF);
            }
            if self.c & FLUSH_SIG2 != 0 {
                self.byte_out(((self.c >> 11) & 0xFF) as u8);
                if ((self.c >> 11) & 0xFF) as u8 == MARKER_ESC {
                    self.byte_out(MARKER_STUFF);
                }
            }
        }
    }

    pub fn into_output(self) -> Vec<u8> {
        self.out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JbigError {
    /// The bitmap is smaller than width x height requires.
    BitmapTooSmall { need: usize, got: usize },
    /// The number of bitmaps does not match the number of bit planes.
    PlaneCount { expected: u8, got: usize },
    /// Width or height was zero.
    Empty,
    /// The image's byte count is not representable on this target.
    GeometryOverflow,
}

impl std::fmt::Display for JbigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JbigError::BitmapTooSmall { need, got } => {
                write!(f, "bitmap too small: need {need} bytes, got {got}")
            }
            JbigError::PlaneCount { expected, got } => {
                write!(f, "{got} bitmap(s) for a {expected}-plane image")
            }
            JbigError::Empty => write!(f, "image has zero width or height"),
            JbigError::GeometryOverflow => write!(f, "image byte count overflows usize"),
        }
    }
}

impl std::error::Error for JbigError {}

/// Encodes a bi-level image into a JBIG bi-level image entity (BIE).
#[derive(Debug, Clone)]
pub struct JbigEncoder {
    width: u32,
    height: u32,
    l0: u32,
    order: u8,
    options: u8,
    /// Bytes per line including padding to a whole byte.
    hbpl: usize,
    stripes: usize,
    /// Bit planes: 1 for the bi-level page the default path sends, 2 for the two-bit page
    /// HP's own driver sends (docs/differences.md item 11).
    planes: u8,
    /// One probability state table per plane, CONTEXTS bytes each.
    st: Box<[u8]>,
}

impl JbigEncoder {
    /// `width` is the image width announced in the BIH and used for the bitmap stride. The
    /// ZjStream path passes the width rounded up to a multiple of 128 here while keeping
    /// the true width in its `RASTER_X` item.
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_options(width, height, DEFAULT_L0, DEFAULT_ORDER, DEFAULT_OPTIONS)
    }

    pub fn with_options(width: u32, height: u32, l0: u32, order: u8, options: u8) -> Self {
        let hbpl = width.div_ceil(8) as usize;
        let l0 = l0.max(1);
        let stripes = if height == 0 {
            0
        } else {
            height.div_ceil(l0) as usize
        };
        Self {
            width,
            height,
            l0,
            order,
            options,
            hbpl,
            stripes,
            planes: 1,
            st: vec![0u8; CONTEXTS].into_boxed_slice(),
        }
    }

    /// Encode this many bit planes instead of one. Each plane is one bit of the pixel
    /// value, most significant plane first: that is what the reference encoder writes when
    /// it is given a PGM and told to use binary words rather than Gray-coded ones.
    pub fn planes(mut self, planes: u8) -> Self {
        let planes = planes.max(1);
        self.planes = planes;
        self.st = vec![0u8; CONTEXTS * planes as usize].into_boxed_slice();
        self
    }

    /// How many bit planes this encoder will write.
    pub fn plane_count(&self) -> u8 {
        self.planes
    }

    pub fn bytes_per_line(&self) -> usize {
        self.hbpl
    }

    pub fn stripes(&self) -> usize {
        self.stripes
    }

    /// The 20-byte bi-level image header.
    pub fn bih(&self) -> [u8; 20] {
        let mut b = [0u8; 20];
        b[0] = 0; // DL: first layer stored
        b[1] = 0; // D: highest layer stored, so a single layer
        b[2] = self.planes;
        b[3] = 0;
        b[4..8].copy_from_slice(&self.width.to_be_bytes());
        b[8..12].copy_from_slice(&self.height.to_be_bytes());
        b[12..16].copy_from_slice(&self.l0.to_be_bytes());
        b[16] = 0; // MX
        b[17] = 0; // MY
        b[18] = self.order;
        b[19] = self.options & 0x7F;
        b
    }

    /// Encode a whole image. `bitmap` holds `height` rows of `bytes_per_line()` bytes,
    /// most significant bit first, set bit = ink.
    pub fn encode(&mut self, bitmap: &[u8]) -> Result<Vec<u8>, JbigError> {
        self.encode_planes(&[bitmap])
    }

    /// The same, for an image of `plane_count()` bit planes: one bitmap per plane, in
    /// plane order, most significant plane first.
    ///
    /// Stripe-major and plane-minor, which is the order the reference encoder writes: every
    /// plane's stripe 0, then every plane's stripe 1, each stripe of each plane terminated
    /// by its own marker. Each plane has its own coder state, carried across its stripes.
    pub fn encode_planes(&mut self, bitmaps: &[&[u8]]) -> Result<Vec<u8>, JbigError> {
        if self.width == 0 || self.height == 0 {
            return Err(JbigError::Empty);
        }
        if bitmaps.len() != self.planes as usize {
            return Err(JbigError::PlaneCount {
                expected: self.planes,
                got: bitmaps.len(),
            });
        }
        let need = self
            .hbpl
            .checked_mul(self.height as usize)
            .ok_or(JbigError::GeometryOverflow)?;
        for bitmap in bitmaps {
            if bitmap.len() < need {
                return Err(JbigError::BitmapTooSmall {
                    need,
                    got: bitmap.len(),
                });
            }
        }
        let mut out = Vec::with_capacity(need / 8 + 64);
        out.extend_from_slice(&self.bih());
        for stripe in 0..self.stripes {
            for (plane, bitmap) in bitmaps.iter().enumerate() {
                out.extend_from_slice(&self.encode_stripe(bitmap, stripe, plane));
            }
        }
        Ok(out)
    }

    fn encode_stripe(&mut self, bitmap: &[u8], stripe: usize, plane: usize) -> Vec<u8> {
        let hbpl = self.hbpl;
        let xd = self.width as usize;
        let yd = self.height as usize;
        let hl = self.l0 as usize;
        let y0 = stripe * hl;
        let reset = stripe == 0;
        let lines = hl.min(yd - y0);

        // T.82 typical prediction (TPBON). One bit per line, coded with a context the
        // standard reserves for it, and a "typical" line is then not coded at all: it is
        // reproduced by the decoder from the line above. The bit sent is not the line's
        // typicality but whether it *changed*, so long runs of identical lines cost one
        // bit each.
        let tp = self.options & OPT_TPBON != 0;
        let lrltwo = self.options & OPT_LRLTWO != 0;
        let tp_cx = if lrltwo { TP_CX_LRLTWO } else { TP_CX_DEFAULT };

        // Read before the coder takes the state table mutably.
        let mut ltp_old = if reset {
            false
        } else {
            self.typical_state_at_stripe_start(bitmap, y0)
        };

        let body = {
            let base = plane * CONTEXTS;
            let mut enc = QmEncoder::new(&mut self.st[base..base + CONTEXTS], !reset);
            for i in 0..lines {
                let y = y0 + i;
                let base = y * hbpl;
                // The neighbour rows only exist when the guards below hold, so the
                // subtractions are done inside those branches rather than up front.
                let fresh = !reset;

                if tp {
                    // Line 0 of the first stripe is compared with a blank line; every
                    // other line is compared with the line above it, including across a
                    // stripe boundary.
                    let typical = if i > 0 || fresh {
                        bitmap[base..base + hbpl] == bitmap[base - hbpl..base]
                    } else {
                        bitmap[base..base + hbpl].iter().all(|&b| b == 0)
                    };
                    enc.encode(tp_cx, u32::from(typical == ltp_old));
                    ltp_old = typical;
                    if typical {
                        continue;
                    }
                }

                let mut line_h1: u32 = 0;
                let mut line_h2: u32 = 0;
                let mut line_h3: u32 = 0;
                if i > 0 || fresh {
                    line_h2 = u32::from(bitmap[base - hbpl]) << 8;
                }
                if i > 1 || fresh {
                    line_h3 = u32::from(bitmap[base - hbpl * 2]) << 8;
                }

                let mut j = 0usize;
                let mut k = 0usize;
                while j < xd {
                    line_h1 |= u32::from(bitmap[base + k]);
                    if j + 8 < hbpl * 8 && (i > 0 || fresh) {
                        line_h2 |= u32::from(bitmap[base - hbpl + k + 1]);
                        if i > 1 || fresh {
                            line_h3 |= u32::from(bitmap[base - hbpl * 2 + k + 1]);
                        }
                    }

                    loop {
                        line_h1 <<= 1;
                        line_h2 <<= 1;
                        line_h3 <<= 1;
                        // The layer being coded is the lowest resolution layer - with no
                        // differential layers it is also the only one - and LRLTWO picks
                        // its template. Neither has an adaptive-template pixel here.
                        let cx = if lrltwo {
                            // Two-line template:
                            //   row y-1: columns x-4..x+1 -> line_h2 bits 14..19
                            //   row y  : columns x-3..x-1 -> line_h1 bits  9..12
                            (((line_h2 >> 10) & 0x3F0) | ((line_h1 >> 9) & 0x00F)) as usize
                        } else {
                            // Three-line template:
                            //   row y-2: columns x+1..x+3 -> line_h3 bits 11..13
                            //   row y-1: columns x-4..x   -> line_h2 bits 14..18
                            //   row y  : columns x-2..x-1 -> line_h1 bits  9..10
                            (((line_h3 >> 8) & 0x380)
                                | ((line_h2 >> 12) & 0x07C)
                                | ((line_h1 >> 9) & 0x003)) as usize
                        };
                        enc.encode(cx, (line_h1 >> 8) & 1);
                        j += 1;
                        if j & 7 == 0 || j >= xd {
                            break;
                        }
                    }
                    k += 1;
                }
            }
            enc.flush();
            enc.into_output()
        };

        // Trailing zero bytes are not part of the stripe data, and a trailing 0xFF needs
        // its stuffing byte back before the terminator.
        let mut body = body;
        while body.last() == Some(&0x00) {
            body.pop();
        }
        if body.last() == Some(&MARKER_ESC) {
            body.push(MARKER_STUFF);
        }
        body.push(MARKER_ESC);
        // SDRST is a library-level switch (JBG_SDRST = 0x200), not a bit in the
        // public BIH options byte. This encoder accepts only that byte, so every
        // stripe uses the normal reset marker.
        body.push(MARKER_SDNORM);
        body
    }

    /// The typicality state a stripe starts in: whether the previous stripe's last line was
    /// itself typical. Carrying it across the boundary is what makes the first line of a
    /// stripe cost a bit only when its typicality actually differs from that.
    fn typical_state_at_stripe_start(&self, bitmap: &[u8], y0: usize) -> bool {
        let hbpl = self.hbpl;
        debug_assert!(
            y0 > 0,
            "stripe 0 starts with the state cleared, not from a line"
        );
        let prev = (y0 - 1) * hbpl;
        if y0 > 1 {
            let above = (y0 - 2) * hbpl;
            bitmap[prev..prev + hbpl] == bitmap[above..above + hbpl]
        } else {
            bitmap[prev..prev + hbpl].iter().all(|&b| b == 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn encode(width: u32, height: u32, bitmap: &[u8]) -> Vec<u8> {
        JbigEncoder::new(width, height).encode(bitmap).unwrap()
    }

    fn encode_with(width: u32, height: u32, options: u8, bitmap: &[u8]) -> Vec<u8> {
        JbigEncoder::with_options(width, height, DEFAULT_L0, DEFAULT_ORDER, options)
            .encode(bitmap)
            .unwrap()
    }

    #[test]
    fn bih_layout_matches_the_standard() {
        let e = JbigEncoder::new(4992, 6400);
        let b = e.bih();
        assert_eq!(b.len(), 20);
        assert_eq!(b[0], 0); // DL
        assert_eq!(b[1], 0); // D, a single layer
        assert_eq!(b[2], 1); // one plane
        assert_eq!(&b[4..8], &4992u32.to_be_bytes());
        assert_eq!(&b[8..12], &6400u32.to_be_bytes());
        // L0 is four bytes, not one: the easiest field to get wrong
        assert_eq!(&b[12..16], &128u32.to_be_bytes());
        assert_eq!(b[16], 0); // MX
        assert_eq!(b[17], 0); // MY
        assert_eq!(b[18], 3); // ILEAVE | SMID
        assert_eq!(b[19], 0x5C); // DPON | TPBON | TPDON | LRLTWO
    }

    #[test]
    fn stripe_count_follows_l0() {
        assert_eq!(JbigEncoder::new(8, 128).stripes(), 1);
        assert_eq!(JbigEncoder::new(8, 129).stripes(), 2);
        assert_eq!(JbigEncoder::new(8, 6400).stripes(), 50);
        assert_eq!(JbigEncoder::new(8, 1).stripes(), 1);
    }

    /// 8x2: row 0 = 10101010, row 1 blank. Byte-exact against the reference encoder, both
    /// for the configuration we ship and for options = 0: those two take different paths
    /// through the template choice and the typical-prediction bit.
    #[test]
    fn golden_8x2() {
        let want = hex("000001000000000800000002000000800000035ce164ff02");
        assert_eq!(encode(8, 2, &[0xAA, 0x00]), want);
        let no_options = hex("0000010000000008000000020000008000000300be80ff02");
        assert_eq!(encode_with(8, 2, 0, &[0xAA, 0x00]), no_options);
    }

    #[test]
    fn high_bih_option_bit_does_not_select_sdrst() {
        let normal = encode_with(8, 2, 0, &[0xAA, 0x00]);
        let high_bit = encode_with(8, 2, 0x80, &[0xAA, 0x00]);
        assert_eq!(high_bit, normal);
    }

    #[test]
    fn extreme_geometry_returns_an_error_on_every_target() {
        assert!(matches!(
            JbigEncoder::new(u32::MAX, u32::MAX).encode(&[]),
            Err(JbigError::GeometryOverflow | JbigError::BitmapTooSmall { .. })
        ));
    }

    /// Two planes: the fixture the reference encoder writes for a PGM of
    /// `0 1 2 3 0 1 2 3 / 3 3 0 0 1 1 2 2` with binary (not Gray-coded) words.
    #[test]
    fn golden_two_planes_8x2() {
        let want = hex("000002000000000800000002000000800000031cc17080ff02d0eaff02");
        // Plane 0 is the high bit of each sample, plane 1 the low bit.
        let plane0 = [0x33u8, 0xC3];
        let plane1 = [0x55u8, 0xCC];
        // Options 0x1c: TPDON | TPBON | DPON, without LRLTWO. That is the configuration
        // the reference encoder picks for a multi-plane image (it sets LRLTWO only for a
        // bi-level one), and it is what the golden above was captured with.
        let got = JbigEncoder::with_options(8, 2, DEFAULT_L0, DEFAULT_ORDER, 0x1C)
            .planes(2)
            .encode_planes(&[&plane0, &plane1])
            .unwrap();
        assert_eq!(got, want);
    }

    /// Two planes across a stripe boundary: 8x130 samples of (x + y) % 4, so 130 lines is a
    /// full stripe plus a two-line one. That is the case a per-plane coder state gets wrong.
    #[test]
    fn golden_two_planes_two_stripes() {
        let want =
            hex("000002000000000800000082000000800000031cc14eea73e05580ff02d0ce1fba64ff02ff02ff02");
        let mut plane0 = vec![0u8; 130];
        let mut plane1 = vec![0u8; 130];
        for (y, (hi, lo)) in plane0.iter_mut().zip(plane1.iter_mut()).enumerate() {
            let mut h = 0u8;
            let mut l = 0u8;
            for x in 0..8u32 {
                let v = (x + y as u32) % 4;
                if (v >> 1) & 1 == 1 {
                    h |= 0x80 >> x;
                }
                if v & 1 == 1 {
                    l |= 0x80 >> x;
                }
            }
            *hi = h;
            *lo = l;
        }
        let got = JbigEncoder::with_options(8, 130, DEFAULT_L0, DEFAULT_ORDER, 0x1C)
            .planes(2)
            .encode_planes(&[&plane0, &plane1])
            .unwrap();
        assert_eq!(got, want);
    }

    /// The number of bitmaps has to match the number of planes the encoder was built for.
    #[test]
    fn plane_count_must_match() {
        let one = [0u8; 2];
        let err = JbigEncoder::new(8, 2)
            .planes(2)
            .encode_planes(&[&one])
            .unwrap_err();
        assert!(
            matches!(
                err,
                JbigError::PlaneCount {
                    expected: 2,
                    got: 1
                }
            ),
            "{err}"
        );
        let err = JbigEncoder::new(8, 2)
            .encode_planes(&[&one, &one])
            .unwrap_err();
        assert!(
            matches!(
                err,
                JbigError::PlaneCount {
                    expected: 1,
                    got: 2
                }
            ),
            "{err}"
        );
        assert_eq!(JbigEncoder::new(8, 2).plane_count(), 1);
        assert_eq!(JbigEncoder::new(8, 2).planes(2).plane_count(), 2);
    }

    /// 16x4 with a denser pattern.
    #[test]
    fn golden_16x4() {
        let want = hex("000001000000001000000004000000800000035cfaa872927a5193ff02");
        let bm = [0xF0, 0x0F, 0x0F, 0xF0, 0xFF, 0x00, 0x00, 0xFF];
        assert_eq!(encode(16, 4, &bm), want);
        let no_options = hex("0000010000000010000000040000008000000300fac68596842853ff02");
        assert_eq!(encode_with(16, 4, 0, &bm), no_options);
    }

    /// 800x600, bands of 17 rows alternating solid and blank: five stripes, so this covers
    /// the stripe loop and the carried probability state between stripes.
    #[test]
    fn golden_800x600_five_stripes() {
        let want = hex(
            "000001000000032000000258000000800000035cfd34d1d9f4629a6729aa86261a03ff02\
             509c758ea77bc922ff009760ff02e7130a84b1336b40065e80ff025e439f22e33e63f2f99fc0ff02\
             121d3fe58c4f9cff02",
        );
        let stride = 100usize; // 800 / 8
        let mut bm = vec![0u8; stride * 600];
        for y in 0..600 {
            if (y / 17) % 2 == 0 {
                for b in &mut bm[y * stride..(y + 1) * stride] {
                    *b = 0xFF;
                }
            }
        }
        assert_eq!(encode(800, 600, &bm), want);
    }

    /// 1x200 single column: a degenerate width, and 200 rows over two stripes.
    #[test]
    fn golden_1x200_single_column() {
        let want = hex(
            "0000010000000001000000c8000000800000035cf303d2e768d6db6db6db6db6db6db6db70ff02\
             6090db6db6db6db6dcff02",
        );
        let bm: Vec<u8> = (0..200)
            .map(|y| if y % 3 == 0 { 0x80 } else { 0x00 })
            .collect();
        assert_eq!(encode(1, 200, &bm), want);
    }

    #[test]
    fn terminator_and_stuffing_are_present() {
        let out = encode(8, 2, &[0xAA, 0x00]);
        // SDNORM terminator closes the only stripe, and no zero padding survives it
        assert_eq!(&out[out.len() - 2..], &[MARKER_ESC, MARKER_SDNORM]);
        assert_eq!(out.len(), 20 + 4);
    }

    #[test]
    fn rejects_bitmaps_that_are_too_small() {
        let mut e = JbigEncoder::new(16, 4);
        assert_eq!(
            e.encode(&[0u8; 4]).unwrap_err(),
            JbigError::BitmapTooSmall { need: 8, got: 4 }
        );
        let mut e = JbigEncoder::new(0, 4);
        assert_eq!(e.encode(&[]).unwrap_err(), JbigError::Empty);
    }

    #[test]
    fn an_all_blank_page_stays_tiny() {
        let out = encode(4992, 6400, &vec![0u8; 624 * 6400]);
        // 50 stripes, each compressing to very little
        assert!(out.len() < 2000, "blank page took {} bytes", out.len());
        assert_eq!(JbigEncoder::new(4992, 6400).stripes(), 50);
    }
}
