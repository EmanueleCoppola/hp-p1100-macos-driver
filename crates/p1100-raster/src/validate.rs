//! The page-level checks every reader and every direct converter applies before a page is
//! handed on.

use crate::color_space;
use crate::error::RasterError;
use crate::page_header::PageHeader;

/// The page-level checks both readers and direct bitmap converters apply before handing a page on.
///
/// cupsCompression is deliberately not one of them. It is only ever populated from the
/// PostScript page device dictionary (cups/raster-interpret.c) and libcups never consults it
/// when reading pixels: page data is always cupsBytesPerLine * cupsHeight raw bytes. Real
/// cgpdftoraster output carries 401 here, which is not a CUPS compression code at all, so
/// trusting it would reject valid jobs. The byte count is the real check, and the pixel
/// conversion fails loudly on a depth it cannot handle.
pub fn validate_page_header(header: &PageHeader) -> Result<(), RasterError> {
    if header.cups_width == 0
        || header.cups_height == 0
        || header.cups_bytes_per_line == 0
        || header.hw_resolution[0] == 0
        || header.hw_resolution[1] == 0
    {
        return Err(RasterError::BadGeometry {
            width: header.cups_width,
            height: header.cups_height,
            bytes_per_line: header.cups_bytes_per_line,
        });
    }
    if !matches!(header.cups_bits_per_pixel, 1 | 8) {
        return Err(RasterError::UnsupportedDepth(header.cups_bits_per_pixel));
    }
    let supported_model = (matches!(header.cups_color_space, color_space::W | color_space::SW)
        && header.cups_bits_per_pixel == 8)
        || (header.cups_color_space == color_space::K && header.cups_bits_per_pixel == 1);
    if !supported_model {
        if !matches!(header.cups_color_space, color_space::W | color_space::K) {
            return Err(RasterError::UnsupportedColorSpace(header.cups_color_space));
        }
        return Err(RasterError::UnsupportedColorModel {
            bits_per_color: header.cups_bits_per_color,
            bits_per_pixel: header.cups_bits_per_pixel,
            num_colors: header.cups_num_colors,
        });
    }
    if header.cups_color_order != 0 {
        return Err(RasterError::UnsupportedColorOrder(header.cups_color_order));
    }
    if header.cups_bits_per_color != header.cups_bits_per_pixel || header.cups_num_colors != 1 {
        return Err(RasterError::UnsupportedColorModel {
            bits_per_color: header.cups_bits_per_color,
            bits_per_pixel: header.cups_bits_per_pixel,
            num_colors: header.cups_num_colors,
        });
    }
    if header.cups_width > u32::MAX - 127 {
        return Err(RasterError::BadGeometry {
            width: header.cups_width,
            height: header.cups_height,
            bytes_per_line: header.cups_bytes_per_line,
        });
    }
    let min_line = (header.cups_width as usize)
        .checked_mul(header.cups_bits_per_pixel as usize)
        .ok_or(RasterError::GeometryOverflow)?
        .div_ceil(8);
    if (header.cups_bytes_per_line as usize) < min_line {
        return Err(RasterError::BadGeometry {
            width: header.cups_width,
            height: header.cups_height,
            bytes_per_line: header.cups_bytes_per_line,
        });
    }
    header.payload_len()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color_space;
    use crate::test_support::letter_mono_header;

    #[test]
    fn rejects_non_grayscale_or_non_chunked_headers() {
        let mut h = letter_mono_header(8, 1);
        h.cups_color_space = color_space::RGB;
        assert!(matches!(
            validate_page_header(&h),
            Err(RasterError::UnsupportedColorSpace(_))
        ));
        h.cups_color_space = color_space::W;
        h.cups_color_order = 1;
        assert!(matches!(
            validate_page_header(&h),
            Err(RasterError::UnsupportedColorOrder(_))
        ));
    }
    #[test]
    fn accepts_screened_grayscale_and_rejects_unsupported_depth_models() {
        let mut h = letter_mono_header(8, 1);
        h.cups_color_space = color_space::SW;
        assert!(validate_page_header(&h).is_ok());
        h.cups_color_space = color_space::W;
        h.cups_bits_per_pixel = 1;
        h.cups_bits_per_color = 1;
        assert!(matches!(
            validate_page_header(&h),
            Err(RasterError::UnsupportedColorModel { .. })
        ));
    }
}
