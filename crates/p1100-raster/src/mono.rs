//! Converting a page's pixels into the 1-bit bitmap the printer is fed.

use crate::error::RasterError;
use crate::page_header::PageHeader;
use crate::validate::validate_page_header;

/// CUPS gray 0 is black and 255 is white, so anything below the midpoint is ink.
pub const DEFAULT_THRESHOLD: u8 = 128;
/// Convert one page of CUPS raster pixels into a 1-bit-per-pixel bitmap.
///
/// A set bit means ink, matching the reference encoder's dot counting. Rows are padded
/// to [`PageHeader::mono_row_bytes`], and only the page's own pixels are read: everything past
/// `cups_width`, whether it is padding inside the last used byte or whole padding bytes from a
/// padded `cups_bytes_per_line`, is left out of the bitmap rather than printed in the margin.
pub fn to_mono1(header: &PageHeader, pixels: &[u8], threshold: u8) -> Result<Vec<u8>, RasterError> {
    validate_page_header(header)?;
    let bitmap_len = header.bitmap_len()?;
    let expected = header.payload_len()?;
    if pixels.len() < expected {
        return Err(RasterError::ShortPixels {
            expected,
            got: pixels.len(),
        });
    }
    let w = header.cups_width as usize;
    let h = header.cups_height as usize;
    let stride = header.mono_row_bytes();
    let src_stride = header.cups_bytes_per_line as usize;
    let mut out = vec![0u8; bitmap_len];
    match header.cups_bits_per_pixel {
        8 => {
            for y in 0..h {
                let src = &pixels[y * src_stride..];
                let dst = &mut out[y * stride..];
                for x in 0..w {
                    if src[x] < threshold {
                        dst[x >> 3] |= 0x80 >> (x & 7);
                    }
                }
            }
        }
        1 => {
            // The page's own bytes only: bits past the width are not part of the page, and a
            // producer that leaves them set must not print a row of dots in the right margin.
            // CUPS pads 1-bit rows out to four bytes, so there is normally padding to be
            // careful about, both whole bytes and the tail of the last used one.
            let page_bytes = w.div_ceil(8);
            let copy = page_bytes.min(src_stride).min(stride);
            for y in 0..h {
                let src = &pixels[y * src_stride..y * src_stride + copy];
                out[y * stride..y * stride + copy].copy_from_slice(src);
                if w & 7 != 0 && copy == page_bytes {
                    out[y * stride + copy - 1] &= !(0xFFu8 >> (w % 8));
                }
            }
        }
        other => return Err(RasterError::UnsupportedDepth(other)),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color_space;
    use crate::test_support::letter_mono_header;

    #[test]
    fn eight_bit_gray_thresholds_to_one_bit_ink() {
        let h = letter_mono_header(8, 2);
        let mut px = vec![255u8; 16];
        px[0] = 0;
        px[1] = 0;
        px[15] = 0;
        let mono = to_mono1(&h, &px, DEFAULT_THRESHOLD).unwrap();
        assert_eq!(mono.len(), 16 * 2);
        assert_eq!(mono[0], 0b1100_0000);
        assert_eq!(mono[16], 0b0000_0001);
    }
    #[test]
    fn one_bit_input_is_copied_and_row_padded() {
        let mut h = letter_mono_header(8, 1);
        h.cups_bits_per_color = 1;
        h.cups_bits_per_pixel = 1;
        h.cups_color_space = color_space::K;
        h.cups_bytes_per_line = 1;
        let mono = to_mono1(&h, &[0b1010_0000], DEFAULT_THRESHOLD).unwrap();
        assert_eq!(mono.len(), 16);
        assert_eq!(mono[0], 0b1010_0000);
        assert!(mono[1..].iter().all(|b| *b == 0), "row pad must be zeroed");
    }
    /// A 1-bit row padded out to four bytes, with the padding full of ink. Those bits are not
    /// part of the page, so they must not reach the bitmap and print in the margin.
    #[test]
    fn one_bit_padding_is_masked_away() {
        let mut h = letter_mono_header(12, 1);
        h.cups_bits_per_color = 1;
        h.cups_bits_per_pixel = 1;
        h.cups_color_space = color_space::K;
        h.cups_bytes_per_line = 4;
        // Twelve page bits: eight in the first byte and the top nibble of the second. The rest
        // of the second byte, and the two whole bytes after it, are padding and set to ink.
        let px = [0b1010_1010u8, 0b1010_1111, 0xFF, 0xFF];
        let mono = to_mono1(&h, &px, DEFAULT_THRESHOLD).unwrap();
        assert_eq!(mono.len(), 16);
        assert_eq!(mono[0], 0b1010_1010);
        assert_eq!(mono[1], 0b1010_0000, "the padding bits must be masked");
    }
    /// 8-bit grey with rows padded past the width, which is not what cgpdftoraster produces -
    /// every page size this PPD declares comes out with bytes/line equal to the width - and so
    /// is exactly the case worth pinning: the padding is not part of the page.
    #[test]
    fn eight_bit_rows_padded_past_the_width_ignore_the_padding() {
        let mut h = letter_mono_header(8, 2);
        h.cups_bytes_per_line = 12;
        let mut px = vec![255u8; 24];
        px[0] = 0; // page pixel (0, 0)
        px[8] = 0; // row 0 padding
        px[12 + 7] = 0; // page pixel (7, 1)
        px[12 + 11] = 0; // row 1 padding
        let mono = to_mono1(&h, &px, DEFAULT_THRESHOLD).unwrap();
        assert_eq!(mono.len(), 32);
        assert_eq!(mono[0], 0b1000_0000);
        assert_eq!(mono[16], 0b0000_0001);
    }
    #[test]
    fn direct_converters_reject_short_pixel_buffers() {
        let h = letter_mono_header(8, 2);
        assert_eq!(
            to_mono1(&h, &[0u8; 8], DEFAULT_THRESHOLD).unwrap_err(),
            RasterError::ShortPixels {
                expected: 16,
                got: 8
            }
        );
    }
}
