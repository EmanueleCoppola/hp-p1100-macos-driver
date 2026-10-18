//! One page's header, and the geometry derived from it.
//!
//! The header is what CUPS hands the filter per page; the derived quantities are what the
//! conversion and compression layers work in: a row stride and a bitmap size aligned the way
//! the reference encoder aligns them.

use crate::color_space;
use crate::compression;
use crate::error::RasterError;
use crate::wire::{MAX_BITMAP_PAYLOAD, MAX_PAGE_PAYLOAD};

#[derive(Debug, Clone, PartialEq)]
pub struct PageHeader {
    pub media_class: String,
    pub media_color: String,
    pub media_type: String,
    pub output_type: String,
    pub hw_resolution: [u32; 2],
    pub margins: [u32; 2],
    pub num_copies: u32,
    pub collate: u32,
    pub duplex: u32,
    pub orientation: u32,
    pub page_size: [u32; 2],
    pub cups_width: u32,
    pub cups_height: u32,
    /// `cupsMediaType`: the media type code the PPD selected. The P1100 PPD maps its
    /// `MediaType` options onto this field (`Paper` -> 1, `TransparencyPaper` -> 2, and so
    /// on), which makes it the authoritative code to forward to the printer rather than
    /// re-deriving it from the option name.
    pub cups_media_type: u32,
    pub cups_bits_per_color: u32,
    pub cups_bits_per_pixel: u32,
    pub cups_bytes_per_line: u32,
    pub cups_color_order: u32,
    pub cups_color_space: u32,
    pub cups_compression: u32,
    pub cups_row_count: u32,
    pub cups_row_feed: u32,
    pub cups_row_step: u32,
    pub cups_num_colors: u32,
    pub cups_borderless_scaling: f32,
    pub cups_page_size: [f32; 2],
    pub cups_imaging_bbox: [f32; 4],
    pub cups_page_size_name: String,
    pub cups_marker_type: String,
    pub cups_rendering_intent: String,
}

impl PageHeader {
    /// Total bytes of pixel data this header accounts for.
    pub fn payload_len(&self) -> Result<usize, RasterError> {
        let size = (self.cups_bytes_per_line as usize)
            .checked_mul(self.cups_height as usize)
            .ok_or(RasterError::GeometryOverflow)?;
        if size > MAX_PAGE_PAYLOAD {
            return Err(RasterError::PayloadTooLarge {
                size,
                max: MAX_PAGE_PAYLOAD,
            });
        }
        Ok(size)
    }

    /// Bytes per row for the 1-bit image handed to the printer.
    ///
    /// The image width is rounded up to a multiple of 128 pixels (the reference encoder
    /// aligns it that way before compression), which also makes every row a multiple of
    /// 16 bytes. Both the compressed and the uncompressed path therefore use the same
    /// stride, so the same bitmap can be fed to either.
    pub fn mono_row_bytes(&self) -> usize {
        (align128(self.cups_width) / 8) as usize
    }

    /// Width rounded up to the 128-pixel boundary used for the JBIG image, while the
    /// encoder's `RASTER_X` item keeps the true width.
    pub fn jbig_width(&self) -> u32 {
        align128(self.cups_width)
    }

    /// Bytes allocated for one converted 1-bit bitmap.
    pub fn bitmap_len(&self) -> Result<usize, RasterError> {
        if self.cups_width > u32::MAX - 127 {
            return Err(RasterError::BadGeometry {
                width: self.cups_width,
                height: self.cups_height,
                bytes_per_line: self.cups_bytes_per_line,
            });
        }
        let size = self
            .mono_row_bytes()
            .checked_mul(self.cups_height as usize)
            .ok_or(RasterError::GeometryOverflow)?;
        if size > MAX_BITMAP_PAYLOAD {
            return Err(RasterError::BitmapTooLarge {
                size,
                max: MAX_BITMAP_PAYLOAD,
            });
        }
        Ok(size)
    }
}
impl Default for PageHeader {
    /// A plausible page, so a test or a caller can set the few fields it cares about: a
    /// 4900x6400 pixel letter page at 600 dpi, eight bits per pixel grey, one colour. The
    /// reader overwrites every field it parses, so this never reaches a real job by accident.
    fn default() -> Self {
        Self {
            media_class: String::new(),
            media_color: String::new(),
            media_type: "Paper".into(),
            output_type: String::new(),
            hw_resolution: [600, 600],
            margins: [12, 12],
            num_copies: 1,
            collate: 0,
            duplex: 0,
            orientation: 0,
            page_size: [612, 792],
            cups_width: 4900,
            cups_height: 6400,
            cups_media_type: 1, // standard paper, the code the PPD gives plain paper
            cups_bits_per_color: 8,
            cups_bits_per_pixel: 8,
            cups_bytes_per_line: 4900,
            cups_color_order: 0,
            cups_color_space: color_space::W,
            cups_compression: compression::NONE,
            cups_row_count: 0,
            cups_row_feed: 0,
            cups_row_step: 0,
            cups_num_colors: 1,
            cups_borderless_scaling: 1.0,
            cups_page_size: [612.0, 792.0],
            cups_imaging_bbox: [12.0, 12.0, 600.0, 780.0],
            cups_page_size_name: "Letter".into(),
            cups_marker_type: String::new(),
            cups_rendering_intent: String::new(),
        }
    }
}

/// Round up to a multiple of 128, the JBIG image width alignment.
pub fn align128(v: u32) -> u32 {
    (v + 127) & !127
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::letter_mono_header;

    #[test]
    fn width_alignment_matches_the_reference_encoder() {
        assert_eq!(align128(4900), 4992);
        assert_eq!(align128(4992), 4992);
        assert_eq!(align128(1), 128);
        assert_eq!(align128(0), 0);
        // 4900 px -> 4992 px -> 624 bytes, which is the reference plane stride.
        assert_eq!(align128(4900) / 8, 624);
    }
    #[test]
    fn mono_geometry_matches_the_reference_encoder() {
        let h = letter_mono_header(4900, 6400);
        assert_eq!(h.mono_row_bytes(), 624);
        assert_eq!(h.jbig_width(), 4992);
        assert_eq!(h.payload_len().unwrap(), 31_360_000);
    }
    #[test]
    fn checks_payload_multiplication_before_the_limit() {
        let h = PageHeader {
            cups_bytes_per_line: u32::MAX,
            cups_height: u32::MAX,
            ..PageHeader::default()
        };
        assert!(matches!(
            h.payload_len(),
            Err(RasterError::PayloadTooLarge { .. }) | Err(RasterError::GeometryOverflow)
        ));
    }
    #[test]
    fn checks_aligned_bitmap_size_before_converter_allocation() {
        let h = PageHeader {
            cups_width: 1,
            cups_height: MAX_PAGE_PAYLOAD as u32,
            cups_bytes_per_line: 1,
            ..PageHeader::default()
        };
        assert!(matches!(
            h.bitmap_len(),
            Err(RasterError::BitmapTooLarge { .. }) | Err(RasterError::GeometryOverflow)
        ));
    }
}
