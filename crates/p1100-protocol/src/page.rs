//! What the caller says about one page: its geometry, its options and its medium.

use crate::{duplex, media, paper, source};

/// Geometry and options for one page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageGeometry {
    /// True pixel width, before JBIG's 128-pixel alignment.
    pub width: u32,
    pub height: u32,
    pub resolution_x: u32,
    pub resolution_y: u32,
    /// 1 for monochrome bilevel.
    pub bits_per_pixel: u32,
    /// Emitted only when non-zero.
    pub offset_x: u32,
    pub offset_y: u32,
    pub paper: u32,
    pub media: u32,
    pub source: u32,
    pub copies: u32,
    pub duplex: u32,
    pub econo_mode: bool,
    pub ret: bool,
}

impl PageGeometry {
    /// Monochrome page at the given resolution, with the defaults the P1102 PPD uses.
    pub fn mono(width: u32, height: u32, resolution: u32) -> Self {
        Self {
            width,
            height,
            resolution_x: resolution,
            resolution_y: resolution,
            bits_per_pixel: 1,
            offset_x: 0,
            offset_y: 0,
            paper: paper::LETTER,
            media: media::STANDARD,
            source: source::ONLY_ONE,
            copies: 1,
            duplex: duplex::OFF,
            econo_mode: false,
            ret: true,
        }
    }
}
