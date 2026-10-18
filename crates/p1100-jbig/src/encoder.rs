//! The bi-level image entity: the header, the stripes, and the context model that turns a
//! bitmap into symbols for the coder.

use crate::error::JbigError;
use crate::qm::{QmEncoder, MARKER_ESC, MARKER_SDNORM, MARKER_STUFF};
use crate::tables::CONTEXTS;

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
mod tests;
