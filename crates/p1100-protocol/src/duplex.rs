//! Duplex modes (`DMDUPLEX`).

pub const OFF: u32 = 1;
pub const LONG_EDGE: u32 = 2;
pub const SHORT_EDGE: u32 = 3;

/// True for the two modes that actually duplex a page.
pub fn is_duplexing(mode: u32) -> bool {
    mode == LONG_EDGE || mode == SHORT_EDGE
}
