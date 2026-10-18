//! Everything that can go wrong reading a raster stream or converting a page.

use crate::wire::SYNC_RASTER3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasterError {
    /// Sync word was not one this reader handles.
    BadSync(u32),
    Truncated,
    /// Decoded geometry does not account for the remaining bytes.
    GeometryMismatch {
        page_index: usize,
        expected: usize,
        remaining: usize,
    },
    /// Depth we cannot convert to 1 bpp.
    UnsupportedDepth(u32),
    /// A tone table shorter than one whole table.
    ToneTableShort {
        need: usize,
        got: usize,
    },
    /// The underlying reader failed.
    Io(String),
    /// Degenerate geometry that cannot be trusted.
    BadGeometry {
        width: u32,
        height: u32,
        bytes_per_line: u32,
    },
    PayloadTooLarge {
        size: usize,
        max: usize,
    },
    BitmapTooLarge {
        size: usize,
        max: usize,
    },
    GeometryOverflow,
    UnsupportedColorSpace(u32),
    UnsupportedColorOrder(u32),
    UnsupportedColorModel {
        bits_per_color: u32,
        bits_per_pixel: u32,
        num_colors: u32,
    },
    ShortPixels {
        expected: usize,
        got: usize,
    },
}

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RasterError::BadSync(v) => write!(
                f,
                "unknown raster sync word {v:#010x}; this reader handles RaS3 ({:#010x})",
                SYNC_RASTER3
            ),
            RasterError::Truncated => write!(f, "raster stream ends in the middle of a page"),
            RasterError::GeometryMismatch {
                page_index,
                expected,
                remaining,
            } => write!(
                f,
                "page {page_index} needs {expected} bytes of pixel data but only {remaining} remain"
            ),
            RasterError::UnsupportedDepth(bits) => {
                write!(f, "unsupported raster depth: {bits} bits per pixel")
            }
            RasterError::ToneTableShort { need, got } => write!(
                f,
                "the tone table is {got} bytes; a whole one is {need}. Write one out with \
                 scripts/hp-tone-table.py --out"
            ),
            RasterError::Io(e) => write!(f, "reading the raster stream: {e}"),
            RasterError::BadGeometry {
                width,
                height,
                bytes_per_line,
            } => write!(
                f,
                "implausible raster geometry: {width}x{height} px, {bytes_per_line} bytes per line"
            ),
            RasterError::PayloadTooLarge { size, max } => {
                write!(f, "raster page payload is {size} bytes; maximum is {max}")
            }
            RasterError::BitmapTooLarge { size, max } => {
                write!(f, "converted bitmap is {size} bytes; maximum is {max}")
            }
            RasterError::GeometryOverflow => write!(f, "raster page geometry overflows usize"),
            RasterError::UnsupportedColorSpace(v) => write!(f, "unsupported raster color space: {v}"),
            RasterError::UnsupportedColorOrder(v) => write!(f, "unsupported raster color order: {v}"),
            RasterError::UnsupportedColorModel { bits_per_color, bits_per_pixel, num_colors } => write!(
                f,
                "unsupported raster color model: {bits_per_color} bits/color, {bits_per_pixel} bits/pixel, {num_colors} colors"
            ),
            RasterError::ShortPixels { expected, got } => {
                write!(f, "converter needs {expected} pixel bytes but got {got}")
            }
        }
    }
}

impl std::error::Error for RasterError {}
