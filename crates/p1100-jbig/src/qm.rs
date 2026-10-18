//! The T.82 QM arithmetic coder, encoder side.

use crate::tables::{CONTEXTS, LSZ, NLPS, NMPS};

pub(crate) const MARKER_STUFF: u8 = 0x00;
pub(crate) const MARKER_ESC: u8 = 0xFF;
pub(crate) const MARKER_SDNORM: u8 = 0x02;
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
