//! CUPS raster reader, and conversion to the 1-bit bitmap that the ZjStream encoder
//! hands to JBIG.
//!
//! This is the input side of our filter: CUPS invokes the printer's raster filter with
//! `application/vnd.cups-raster` on stdin, which is exactly where HP's `rastertozjs`
//! sits.
//!
//! Stream layout
//! -------------
//! ```text
//! u32 sync word                  once per stream, not per page
//! cups_page_header2_t            1796 bytes, per page
//! pixel data                     cupsBytesPerLine * cupsHeight bytes
//! cups_page_header2_t            next page, immediately after the previous pixels
//! ...
//! ```
//!
//! The sync word lives in `cupsRasterOpenIO`, which writes it when the stream is opened
//! and reads it back once (`cups/raster-stream.c`), so there is exactly one per stream.
//! `CUPS_RASTER_SYNC` is `0x52615333` (`"RaS3"`) and is the format `cgpdftoraster`
//! produces here; Apple also defines `CUPS_RASTER_SYNCapple` `0x554E4952` (`"UNIR"`).
//!
//! Field offsets are `offsetof()` into CUPS's current `cups_page_header2_t`, which is
//! why the struct has no `Magic`/`Version`/`HeaderSize` members: the sync word carries
//! that role and is stored out of band. Verified against the real header definition and
//! against captured output; see `docs/protocol.md`.

use std::io::Read;

/// `CUPS_RASTER_SYNC`, `"RaS3"`.
pub const SYNC_RASTER3: u32 = 0x5261_5333;
/// `CUPS_RASTER_SYNCv2`, `"RaS2"` (also `CUPS_RASTER_SYNC_PWG`).
pub const SYNC_RASTER2: u32 = 0x5261_5332;
/// `CUPS_RASTER_SYNCv1`, `"RaSt"`.
pub const SYNC_RASTER1: u32 = 0x5261_5374;
/// `CUPS_RASTER_SYNCapple`, `"UNIR"`.
pub const SYNC_APPLE: u32 = 0x5545_4952;

/// `sizeof(cups_page_header2_t)` as measured against CUPS's current definition.
pub const PAGE_HEADER_LEN: usize = 1796;
/// Maximum raw pixel payload accepted for one page as a resource safety bound, not a printer limit.
pub const MAX_PAGE_PAYLOAD: usize = 128 * 1024 * 1024;
/// Maximum allocated bitmap size for one conversion (two planes count together). This is a
/// resource safety bound, not a printer limit.
pub const MAX_BITMAP_PAYLOAD: usize = 128 * 1024 * 1024;

/// `offsetof()` values into `cups_page_header2_t`.
pub mod field {
    pub const MEDIA_CLASS: usize = 0;
    pub const MEDIA_COLOR: usize = 64;
    pub const MEDIA_TYPE: usize = 128;
    pub const OUTPUT_TYPE: usize = 192;
    pub const ADVANCE_DISTANCE: usize = 256;
    pub const COLLATE: usize = 264;
    pub const DUPLEX: usize = 272;
    pub const HW_RESOLUTION: usize = 276;
    pub const IMAGING_BOUNDING_BOX: usize = 284;
    pub const MARGINS: usize = 312;
    pub const MANUAL_FEED: usize = 320;
    pub const NUM_COPIES: usize = 340;
    pub const ORIENTATION: usize = 344;
    pub const PAGE_SIZE: usize = 352;
    pub const CUPS_WIDTH: usize = 372;
    pub const CUPS_HEIGHT: usize = 376;
    pub const CUPS_MEDIA_TYPE: usize = 380;
    pub const CUPS_BITS_PER_COLOR: usize = 384;
    pub const CUPS_BITS_PER_PIXEL: usize = 388;
    pub const CUPS_BYTES_PER_LINE: usize = 392;
    pub const CUPS_COLOR_ORDER: usize = 396;
    pub const CUPS_COLOR_SPACE: usize = 400;
    pub const CUPS_COMPRESSION: usize = 404;
    pub const CUPS_ROW_COUNT: usize = 408;
    pub const CUPS_ROW_FEED: usize = 412;
    pub const CUPS_ROW_STEP: usize = 416;
    pub const CUPS_NUM_COLORS: usize = 420;
    pub const CUPS_BORDERLESS_SCALING: usize = 424;
    pub const CUPS_PAGE_SIZE: usize = 428;
    pub const CUPS_IMAGING_BBOX: usize = 436;
    pub const CUPS_INTEGER: usize = 452;
    pub const CUPS_REAL: usize = 516;
    pub const CUPS_STRING: usize = 580;
    pub const CUPS_MARKER_TYPE: usize = 1604;
    pub const CUPS_RENDERING_INTENT: usize = 1668;
    pub const CUPS_PAGE_SIZE_NAME: usize = 1732;
}

/// `cupsCompression` values.
pub mod compression {
    pub const NONE: u32 = 0;
    pub const RLE: u32 = 1;
    pub const DEFLATE: u32 = 2;
    pub const GZIP: u32 = 3;
    pub const LZW: u32 = 4;
    pub const JPEG: u32 = 5;
    pub const PNG: u32 = 6;
}

/// `cupsColorSpace` values we care about.
pub mod color_space {
    pub const W: u32 = 0;
    pub const RGB: u32 = 1;
    pub const K: u32 = 3;
    pub const SW: u32 = 18;
}

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

/// Iterates the pages of a CUPS raster stream.
pub struct RasterReader<'a> {
    data: &'a [u8],
    pos: usize,
    page_index: usize,
    started: bool,
}

impl<'a> RasterReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            page_index: 0,
            started: false,
        }
    }

    /// Next page as `(header, pixel bytes)`. `Ok(None)` at end of stream.
    pub fn next_page(&mut self) -> Result<Option<(PageHeader, &'a [u8])>, RasterError> {
        if !self.started {
            if self.data.len() < 4 {
                return Err(RasterError::Truncated);
            }
            // The writer stored the sync word as a native u32 (cupsRasterOpenIO), so read
            // it the same way and compare against the formats we understand.
            let sync = u32::from_ne_bytes([self.data[0], self.data[1], self.data[2], self.data[3]]);
            if sync != SYNC_RASTER3 {
                return Err(RasterError::BadSync(sync));
            }
            self.pos = 4;
            self.started = true;
        }
        if self.pos >= self.data.len() {
            return Ok(None);
        }
        let hdr_end = self.pos + PAGE_HEADER_LEN;
        if hdr_end > self.data.len() {
            return Err(RasterError::Truncated);
        }
        let header = parse_header(&self.data[self.pos..hdr_end]);

        validate_page_header(&header)?;
        let payload_len = header.payload_len()?;
        let payload_end = hdr_end + payload_len;
        if payload_end > self.data.len() {
            return Err(RasterError::GeometryMismatch {
                page_index: self.page_index,
                expected: payload_len,
                remaining: self.data.len() - hdr_end,
            });
        }
        let pixels = &self.data[hdr_end..payload_end];
        self.pos = payload_end;
        self.page_index += 1;
        Ok(Some((header, pixels)))
    }
}

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

/// Fill buf, returning how many bytes actually arrived. Fewer than buf.len() means the stream
/// ended, which callers have to tell apart from an error.
fn read_full<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<usize, RasterError> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(RasterError::Io(e.to_string())),
        }
    }
    Ok(filled)
}

/// Streaming CUPS raster reader, for the filter.
///
/// A filter must not hold a whole job in memory: at 600 dpi one Letter page is 31 MB of
/// raster, so a fifty page document would be over a gigabyte, while the compressed output for
/// the same job is a few kilobytes. This reads one page at a time into a buffer the caller
/// reuses, so peak memory is a page rather than a job.
///
/// RasterReader remains for callers that already hold the bytes, such as the inspection
/// tooling. Both share the header parsing and the page validation above.
pub struct RasterStream<R: Read> {
    inner: R,
    header: Vec<u8>,
    page_index: usize,
    started: bool,
}

impl<R: Read> RasterStream<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            header: vec![0u8; PAGE_HEADER_LEN],
            page_index: 0,
            started: false,
        }
    }

    /// Read the next page's header and pixels. payload is replaced with the pixel data.
    /// Ok(None) means the stream ended cleanly between pages.
    pub fn next_page(&mut self, payload: &mut Vec<u8>) -> Result<Option<PageHeader>, RasterError> {
        if !self.started {
            let mut sync = [0u8; 4];
            if read_full(&mut self.inner, &mut sync)? < 4 {
                return Err(RasterError::Truncated);
            }
            // The writer stored the sync word as a native u32 (cupsRasterOpenIO).
            let sync = u32::from_ne_bytes(sync);
            if sync != SYNC_RASTER3 {
                return Err(RasterError::BadSync(sync));
            }
            self.started = true;
        }

        let got = read_full(&mut self.inner, &mut self.header)?;
        if got == 0 {
            return Ok(None);
        }
        if got < PAGE_HEADER_LEN {
            return Err(RasterError::Truncated);
        }

        let header = parse_header(&self.header);
        validate_page_header(&header)?;
        let payload_len = header.payload_len()?;
        payload.clear();
        payload.resize(payload_len, 0);
        let got = read_full(&mut self.inner, payload)?;
        if got < payload_len {
            return Err(RasterError::GeometryMismatch {
                page_index: self.page_index,
                expected: payload_len,
                remaining: got,
            });
        }
        self.page_index += 1;
        Ok(Some(header))
    }
}

fn parse_header(h: &[u8]) -> PageHeader {
    PageHeader {
        media_class: cstr(h, field::MEDIA_CLASS),
        media_color: cstr(h, field::MEDIA_COLOR),
        media_type: cstr(h, field::MEDIA_TYPE),
        output_type: cstr(h, field::OUTPUT_TYPE),
        hw_resolution: [
            u32_at(h, field::HW_RESOLUTION),
            u32_at(h, field::HW_RESOLUTION + 4),
        ],
        margins: [u32_at(h, field::MARGINS), u32_at(h, field::MARGINS + 4)],
        num_copies: u32_at(h, field::NUM_COPIES),
        collate: u32_at(h, field::COLLATE),
        duplex: u32_at(h, field::DUPLEX),
        orientation: u32_at(h, field::ORIENTATION),
        page_size: [u32_at(h, field::PAGE_SIZE), u32_at(h, field::PAGE_SIZE + 4)],
        cups_width: u32_at(h, field::CUPS_WIDTH),
        cups_height: u32_at(h, field::CUPS_HEIGHT),
        cups_media_type: u32_at(h, field::CUPS_MEDIA_TYPE),
        cups_bits_per_color: u32_at(h, field::CUPS_BITS_PER_COLOR),
        cups_bits_per_pixel: u32_at(h, field::CUPS_BITS_PER_PIXEL),
        cups_bytes_per_line: u32_at(h, field::CUPS_BYTES_PER_LINE),
        cups_color_order: u32_at(h, field::CUPS_COLOR_ORDER),
        cups_color_space: u32_at(h, field::CUPS_COLOR_SPACE),
        cups_compression: u32_at(h, field::CUPS_COMPRESSION),
        cups_row_count: u32_at(h, field::CUPS_ROW_COUNT),
        cups_row_feed: u32_at(h, field::CUPS_ROW_FEED),
        cups_row_step: u32_at(h, field::CUPS_ROW_STEP),
        cups_num_colors: u32_at(h, field::CUPS_NUM_COLORS),
        cups_borderless_scaling: f32_at(h, field::CUPS_BORDERLESS_SCALING),
        cups_page_size: [
            f32_at(h, field::CUPS_PAGE_SIZE),
            f32_at(h, field::CUPS_PAGE_SIZE + 4),
        ],
        cups_imaging_bbox: [
            f32_at(h, field::CUPS_IMAGING_BBOX),
            f32_at(h, field::CUPS_IMAGING_BBOX + 4),
            f32_at(h, field::CUPS_IMAGING_BBOX + 8),
            f32_at(h, field::CUPS_IMAGING_BBOX + 12),
        ],
        cups_page_size_name: cstr(h, field::CUPS_PAGE_SIZE_NAME),
        cups_marker_type: cstr(h, field::CUPS_MARKER_TYPE),
        cups_rendering_intent: cstr(h, field::CUPS_RENDERING_INTENT),
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn cstr(b: &[u8], at: usize) -> String {
    const LEN: usize = 64;
    let slice = &b[at..(at + LEN).min(b.len())];
    let end = slice.iter().position(|c| *c == 0).unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..end]).into_owned()
}

/// Round up to a multiple of 128, the JBIG image width alignment.
pub fn align128(v: u32) -> u32 {
    (v + 127) & !127
}

/// CUPS gray 0 is black and 255 is white, so anything below the midpoint is ink.
pub const DEFAULT_THRESHOLD: u8 = 128;

/// Serialise a `PageHeader` into the 1796-byte on-the-wire form.
///
/// Useful for tooling and for generating synthetic rasters in tests; the reader above is
/// the inverse. Unset fields are written as zero. All 64-byte string members are
/// NUL-filled and truncated to fit.
pub fn encode_page_header(h: &PageHeader) -> [u8; PAGE_HEADER_LEN] {
    let mut out = [0u8; PAGE_HEADER_LEN];
    put_cstr(&mut out, field::MEDIA_CLASS, &h.media_class);
    put_cstr(&mut out, field::MEDIA_COLOR, &h.media_color);
    put_cstr(&mut out, field::MEDIA_TYPE, &h.media_type);
    put_cstr(&mut out, field::OUTPUT_TYPE, &h.output_type);
    put_u32(&mut out, field::COLLATE, h.collate);
    put_u32(&mut out, field::DUPLEX, h.duplex);
    put_u32(&mut out, field::HW_RESOLUTION, h.hw_resolution[0]);
    put_u32(&mut out, field::HW_RESOLUTION + 4, h.hw_resolution[1]);
    put_u32(&mut out, field::MARGINS, h.margins[0]);
    put_u32(&mut out, field::MARGINS + 4, h.margins[1]);
    put_u32(&mut out, field::NUM_COPIES, h.num_copies);
    put_u32(&mut out, field::ORIENTATION, h.orientation);
    put_u32(&mut out, field::PAGE_SIZE, h.page_size[0]);
    put_u32(&mut out, field::PAGE_SIZE + 4, h.page_size[1]);
    put_u32(&mut out, field::CUPS_WIDTH, h.cups_width);
    put_u32(&mut out, field::CUPS_HEIGHT, h.cups_height);
    put_u32(&mut out, field::CUPS_MEDIA_TYPE, h.cups_media_type);
    put_u32(&mut out, field::CUPS_BITS_PER_COLOR, h.cups_bits_per_color);
    put_u32(&mut out, field::CUPS_BITS_PER_PIXEL, h.cups_bits_per_pixel);
    put_u32(&mut out, field::CUPS_BYTES_PER_LINE, h.cups_bytes_per_line);
    put_u32(&mut out, field::CUPS_COLOR_ORDER, h.cups_color_order);
    put_u32(&mut out, field::CUPS_COLOR_SPACE, h.cups_color_space);
    put_u32(&mut out, field::CUPS_COMPRESSION, h.cups_compression);
    put_u32(&mut out, field::CUPS_ROW_COUNT, h.cups_row_count);
    put_u32(&mut out, field::CUPS_ROW_FEED, h.cups_row_feed);
    put_u32(&mut out, field::CUPS_ROW_STEP, h.cups_row_step);
    put_u32(&mut out, field::CUPS_NUM_COLORS, h.cups_num_colors);
    put_f32(
        &mut out,
        field::CUPS_BORDERLESS_SCALING,
        h.cups_borderless_scaling,
    );
    put_f32(&mut out, field::CUPS_PAGE_SIZE, h.cups_page_size[0]);
    put_f32(&mut out, field::CUPS_PAGE_SIZE + 4, h.cups_page_size[1]);
    for i in 0..4 {
        put_f32(
            &mut out,
            field::CUPS_IMAGING_BBOX + 4 * i,
            h.cups_imaging_bbox[i],
        );
    }
    put_cstr(&mut out, field::CUPS_MARKER_TYPE, &h.cups_marker_type);
    put_cstr(
        &mut out,
        field::CUPS_RENDERING_INTENT,
        &h.cups_rendering_intent,
    );
    put_cstr(&mut out, field::CUPS_PAGE_SIZE_NAME, &h.cups_page_size_name);
    out
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_ne_bytes());
}

fn put_f32(b: &mut [u8], at: usize, v: f32) {
    b[at..at + 4].copy_from_slice(&v.to_ne_bytes());
}

fn put_cstr(b: &mut [u8], at: usize, v: &str) {
    let bytes = v.as_bytes();
    let n = bytes.len().min(63);
    b[at..at + n].copy_from_slice(&bytes[..n]);
    b[at + n] = 0;
}

/// How many bytes a tone table takes: 256 grey levels, each a 12x12 cell of 2-bit values,
/// four cells per byte. `scripts/hp-tone-table.py --out` writes files of exactly this size,
/// read out of the installed HP driver.
pub const TONE_TABLE_LEN: usize = 256 * TONE_CELL * TONE_ROW_BYTES;

/// The cell's width and height, in pixels.
pub const TONE_CELL: usize = 12;

/// Bytes per cell row: twelve 2-bit values, padded to four bytes.
pub const TONE_ROW_BYTES: usize = 4;

/// One pixel's level, with HP's driver's indexing.
///
/// The table is indexed by the grey value first, then by the pixel's place in the cell: the
/// byte by its group of four columns and the 2-bit field inside it by its position in that
/// group, and the row by `(-y) % 12` - the driver's line counter counts down from the page
/// height rounded up to a multiple of 12, so the cell is walked upwards as the page is walked
/// downwards. A level of 0 is black and 3 is white, the way the tables are built.
pub fn tone_level(table: &[u8], grey: u8, x: usize, y: usize) -> u8 {
    let base = grey as usize * TONE_CELL * TONE_ROW_BYTES;
    let row = (TONE_CELL - y % TONE_CELL) % TONE_CELL;
    let byte = table[base + row * TONE_ROW_BYTES + (x % TONE_CELL) / 4];
    (byte >> (6 - 2 * (x % 4))) & 3
}

/// Screen one page of 8-bit grey into two bit planes, the way HP's driver does.
///
/// Each plane is one bit of the level, most significant plane first, packed with a set bit
/// meaning the level's bit is 1 - which is what the reference encoder writes when it splits a
/// two-bit PGM, so a two-bit page fed through it and through this function comes out the same.
/// Rows are padded to [`PageHeader::mono_row_bytes`] like the one-bit path's.
pub fn to_two_bit_planes(
    header: &PageHeader,
    pixels: &[u8],
    table: &[u8],
) -> Result<[Vec<u8>; 2], RasterError> {
    if table.len() < TONE_TABLE_LEN {
        return Err(RasterError::ToneTableShort {
            need: TONE_TABLE_LEN,
            got: table.len(),
        });
    }
    validate_page_header(header)?;
    if header.cups_bits_per_pixel != 8 {
        return Err(RasterError::UnsupportedDepth(header.cups_bits_per_pixel));
    }
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
    let total = bitmap_len
        .checked_mul(2)
        .ok_or(RasterError::GeometryOverflow)?;
    if total > MAX_BITMAP_PAYLOAD {
        return Err(RasterError::BitmapTooLarge {
            size: total,
            max: MAX_BITMAP_PAYLOAD,
        });
    }
    let mut planes = [vec![0u8; bitmap_len], vec![0u8; bitmap_len]];
    for y in 0..h {
        let src = &pixels[y * src_stride..];
        for x in 0..w {
            let level = tone_level(table, src[x], x, y);
            if level & 2 != 0 {
                planes[0][y * stride + (x >> 3)] |= 0x80 >> (x & 7);
            }
            if level & 1 != 0 {
                planes[1][y * stride + (x >> 3)] |= 0x80 >> (x & 7);
            }
        }
        // Everything past the page width takes the lightest level, which is what the one-bit
        // path's zeroed padding bits mean there: the printer draws the whole aligned width, so
        // a zeroed two-bit margin would print a black stripe down the right-hand side.
        for x in w..stride * 8 {
            planes[0][y * stride + (x >> 3)] |= 0x80 >> (x & 7);
            planes[1][y * stride + (x >> 3)] |= 0x80 >> (x & 7);
        }
    }
    Ok(planes)
}

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

    fn header(width: u32, height: u32, bpl: u32, bpp: u32, name: &str) -> Vec<u8> {
        let mut h = vec![0u8; PAGE_HEADER_LEN];
        h[field::MEDIA_TYPE..field::MEDIA_TYPE + 5].copy_from_slice(b"Paper");
        h[field::HW_RESOLUTION..field::HW_RESOLUTION + 4].copy_from_slice(&600u32.to_ne_bytes());
        h[field::HW_RESOLUTION + 4..field::HW_RESOLUTION + 8]
            .copy_from_slice(&600u32.to_ne_bytes());
        h[field::MARGINS..field::MARGINS + 4].copy_from_slice(&12u32.to_ne_bytes());
        h[field::MARGINS + 4..field::MARGINS + 8].copy_from_slice(&12u32.to_ne_bytes());
        h[field::NUM_COPIES..field::NUM_COPIES + 4].copy_from_slice(&1u32.to_ne_bytes());
        h[field::PAGE_SIZE..field::PAGE_SIZE + 4].copy_from_slice(&612u32.to_ne_bytes());
        h[field::PAGE_SIZE + 4..field::PAGE_SIZE + 8].copy_from_slice(&792u32.to_ne_bytes());
        h[field::CUPS_WIDTH..field::CUPS_WIDTH + 4].copy_from_slice(&width.to_ne_bytes());
        h[field::CUPS_HEIGHT..field::CUPS_HEIGHT + 4].copy_from_slice(&height.to_ne_bytes());
        h[field::CUPS_BITS_PER_COLOR..field::CUPS_BITS_PER_COLOR + 4]
            .copy_from_slice(&bpp.to_ne_bytes());
        h[field::CUPS_BITS_PER_PIXEL..field::CUPS_BITS_PER_PIXEL + 4]
            .copy_from_slice(&bpp.to_ne_bytes());
        h[field::CUPS_BYTES_PER_LINE..field::CUPS_BYTES_PER_LINE + 4]
            .copy_from_slice(&bpl.to_ne_bytes());
        h[field::CUPS_NUM_COLORS..field::CUPS_NUM_COLORS + 4].copy_from_slice(&1u32.to_ne_bytes());
        h[field::CUPS_BORDERLESS_SCALING..field::CUPS_BORDERLESS_SCALING + 4]
            .copy_from_slice(&1.0f32.to_ne_bytes());
        h[field::CUPS_PAGE_SIZE..field::CUPS_PAGE_SIZE + 4]
            .copy_from_slice(&612.0f32.to_ne_bytes());
        h[field::CUPS_PAGE_SIZE + 4..field::CUPS_PAGE_SIZE + 8]
            .copy_from_slice(&792.0f32.to_ne_bytes());
        h[field::CUPS_PAGE_SIZE_NAME..field::CUPS_PAGE_SIZE_NAME + name.len()]
            .copy_from_slice(name.as_bytes());
        h
    }

    fn stream_pages(pages: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut v = SYNC_RASTER3.to_ne_bytes().to_vec();
        for (h, p) in pages {
            v.extend_from_slice(h);
            v.extend_from_slice(p);
        }
        v
    }

    #[test]
    fn header_size_matches_the_cups_definition() {
        // Last member is cupsPageSizeName[64], ending at 1732 + 64.
        assert_eq!(field::CUPS_PAGE_SIZE_NAME + 64, PAGE_HEADER_LEN);
        // Four 64-byte strings, then the rest of the version-2 members in order.
        assert_eq!(field::MEDIA_COLOR, field::MEDIA_CLASS + 64);
        assert_eq!(field::MEDIA_TYPE, field::MEDIA_COLOR + 64);
        assert_eq!(field::OUTPUT_TYPE, field::MEDIA_TYPE + 64);
        assert_eq!(field::ADVANCE_DISTANCE, field::OUTPUT_TYPE + 64);
        assert_eq!(field::CUPS_STRING, field::CUPS_REAL + 16 * 4);
        assert_eq!(field::CUPS_STRING + 16 * 64, field::CUPS_MARKER_TYPE);
        assert_eq!(field::CUPS_MARKER_TYPE + 64, field::CUPS_RENDERING_INTENT);
        assert_eq!(
            field::CUPS_RENDERING_INTENT + 64,
            field::CUPS_PAGE_SIZE_NAME
        );
        assert_eq!(field::CUPS_NUM_COLORS, field::CUPS_ROW_STEP + 4);
        assert_eq!(field::CUPS_BORDERLESS_SCALING, field::CUPS_NUM_COLORS + 4);
        assert_eq!(field::CUPS_PAGE_SIZE, field::CUPS_BORDERLESS_SCALING + 4);
        assert_eq!(field::CUPS_IMAGING_BBOX, field::CUPS_PAGE_SIZE + 8);
        assert_eq!(field::CUPS_INTEGER, field::CUPS_IMAGING_BBOX + 16);
        assert_eq!(field::CUPS_REAL, field::CUPS_INTEGER + 16 * 4);
        // After PageSize[2] come Separations, TraySwitch, Tumble, then cupsWidth.
        assert_eq!(field::CUPS_WIDTH, field::PAGE_SIZE + 20);
    }

    #[test]
    fn reads_one_page_and_lands_on_eof() {
        let h = header(8, 2, 8, 8, "Letter");
        let s = stream_pages(&[(&h, &[0u8; 16])]);
        let mut r = RasterReader::new(&s);
        let (hdr, px) = r.next_page().unwrap().unwrap();
        assert_eq!(hdr.cups_width, 8);
        assert_eq!(hdr.cups_height, 2);
        assert_eq!(hdr.cups_bytes_per_line, 8);
        assert_eq!(hdr.cups_num_colors, 1);
        assert_eq!(hdr.media_type, "Paper");
        assert_eq!(hdr.cups_page_size_name, "Letter");
        assert_eq!(hdr.hw_resolution, [600, 600]);
        assert_eq!(hdr.margins, [12, 12]);
        assert_eq!(hdr.cups_page_size, [612.0, 792.0]);
        assert_eq!(px.len(), 16);
        assert!(r.next_page().unwrap().is_none());
    }

    #[test]
    fn reads_multiple_pages_with_one_sync_word() {
        let h = header(8, 2, 8, 8, "Letter");
        let s = stream_pages(&[(&h, &[0u8; 16]), (&h, &[7u8; 16]), (&h, &[9u8; 16])]);
        // cupsRasterOpenIO writes the sync word once when the stream is opened, so it must
        // appear exactly once no matter how many pages follow.
        let sync = SYNC_RASTER3.to_ne_bytes();
        assert_eq!(
            s.windows(4).filter(|w| **w == sync).count(),
            1,
            "sync word must be written once per stream, not once per page"
        );
        let mut r = RasterReader::new(&s);
        let mut pages = 0;
        while r.next_page().unwrap().is_some() {
            pages += 1;
        }
        assert_eq!(pages, 3);
    }

    #[test]
    fn rejects_a_bad_sync_word() {
        let mut s = vec![0u8; 4];
        s.extend_from_slice(&[0u8; PAGE_HEADER_LEN]);
        let mut r = RasterReader::new(&s);
        assert_eq!(r.next_page().unwrap_err(), RasterError::BadSync(0));
    }

    #[test]
    fn compression_field_is_informational_only() {
        // Real cgpdftoraster output on this machine carries cupsCompression = 401, which is
        // not a valid CUPS code. It must not stop us reading an otherwise valid page.
        let mut h = header(8, 2, 8, 8, "Letter");
        h[field::CUPS_COMPRESSION..field::CUPS_COMPRESSION + 4]
            .copy_from_slice(&401u32.to_ne_bytes());
        let s = stream_pages(&[(&h, &[0u8; 16])]);
        let mut r = RasterReader::new(&s);
        let (hdr, px) = r.next_page().unwrap().unwrap();
        assert_eq!(hdr.cups_compression, 401);
        assert_eq!(px.len(), 16);
    }

    #[test]
    fn fails_loudly_when_geometry_does_not_fit_the_data() {
        let h = header(8, 2, 8, 8, "Letter");
        let s = stream_pages(&[(&h, &[0u8; 8])]);
        let mut r = RasterReader::new(&s);
        match r.next_page().unwrap_err() {
            RasterError::GeometryMismatch {
                expected,
                remaining,
                ..
            } => {
                assert_eq!(expected, 16);
                assert_eq!(remaining, 8);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn rejects_degenerate_geometry() {
        let h = header(0, 2, 8, 8, "Letter");
        let s = stream_pages(&[(&h, &[0u8; 0])]);
        let mut r = RasterReader::new(&s);
        assert!(matches!(
            r.next_page().unwrap_err(),
            RasterError::BadGeometry { .. }
        ));
    }

    #[test]
    fn rejects_a_bytes_per_line_too_small_for_the_width() {
        let h = header(100, 2, 8, 8, "Letter");
        let s = stream_pages(&[(&h, &[0u8; 16])]);
        let mut r = RasterReader::new(&s);
        assert!(matches!(
            r.next_page().unwrap_err(),
            RasterError::BadGeometry { .. }
        ));
    }

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
    fn header_encoder_round_trips_through_the_reader() {
        let h = letter_mono_header(8, 2);
        let bytes = encode_page_header(&h);
        let s = stream_pages(&[(&bytes, &[0u8; 16])]);
        let mut r = RasterReader::new(&s);
        let (got, px) = r.next_page().unwrap().unwrap();
        assert_eq!(got, h);
        assert_eq!(px.len(), 16);
    }
    #[test]
    fn the_streaming_reader_matches_the_slice_reader() {
        let h = header(8, 2, 8, 8, "Letter");
        let s = stream_pages(&[(&h, &[1u8; 16]), (&h, &[2u8; 16]), (&h, &[3u8; 16])]);

        let mut from_slice = Vec::new();
        let mut reader = RasterReader::new(&s);
        while let Some((hdr, px)) = reader.next_page().unwrap() {
            from_slice.push((hdr, px.to_vec()));
        }

        let mut from_stream = Vec::new();
        let mut stream = RasterStream::new(&s[..]);
        let mut buf = Vec::new();
        while let Some(hdr) = stream.next_page(&mut buf).unwrap() {
            from_stream.push((hdr, buf.clone()));
        }

        assert_eq!(from_slice.len(), 3);
        assert_eq!(from_slice, from_stream);
    }

    #[test]
    fn the_streaming_reader_ends_cleanly_between_pages() {
        let h = header(8, 2, 8, 8, "Letter");
        let s = stream_pages(&[(&h, &[0u8; 16])]);
        let mut stream = RasterStream::new(&s[..]);
        let mut buf = Vec::new();
        assert!(stream.next_page(&mut buf).unwrap().is_some());
        assert!(stream.next_page(&mut buf).unwrap().is_none());
    }

    #[test]
    fn the_streaming_reader_reports_a_page_that_ends_early() {
        let h = header(8, 2, 8, 8, "Letter");
        let mut s = stream_pages(&[(&h, &[0u8; 16])]);
        s.truncate(s.len() - 4);
        let mut stream = RasterStream::new(&s[..]);
        let mut buf = Vec::new();
        match stream.next_page(&mut buf).unwrap_err() {
            RasterError::GeometryMismatch {
                expected,
                remaining,
                ..
            } => {
                assert_eq!(expected, 16);
                assert_eq!(remaining, 12);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn the_streaming_reader_handles_short_and_interrupted_reads() {
        struct Chunked {
            data: Vec<u8>,
            pos: usize,
            interrupted: bool,
        }
        impl Read for Chunked {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                let n = out.len().min(3).min(self.data.len() - self.pos);
                out[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
                self.pos += n;
                Ok(n)
            }
        }
        let h = header(8, 2, 8, 8, "Letter");
        let data = stream_pages(&[(&h, &[0u8; 16])]);
        let mut stream = RasterStream::new(Chunked {
            data,
            pos: 0,
            interrupted: false,
        });
        let mut payload = Vec::new();
        assert!(stream.next_page(&mut payload).unwrap().is_some());
        assert_eq!(payload, vec![0; 16]);
        assert!(stream.next_page(&mut payload).unwrap().is_none());
    }

    #[test]
    fn the_streaming_reader_rejects_an_empty_or_misidentified_stream() {
        let empty: Vec<u8> = Vec::new();
        let mut stream = RasterStream::new(&empty[..]);
        let mut buf = Vec::new();
        assert_eq!(
            stream.next_page(&mut buf).unwrap_err(),
            RasterError::Truncated
        );

        let junk = [0u8; 8];
        let mut stream = RasterStream::new(&junk[..]);
        assert_eq!(
            stream.next_page(&mut buf).unwrap_err(),
            RasterError::BadSync(0)
        );
    }

    fn letter_mono_header(width: u32, height: u32) -> PageHeader {
        PageHeader {
            cups_width: width,
            cups_height: height,
            cups_bytes_per_line: width,
            ..PageHeader::default()
        }
    }

    #[test]
    fn mono_geometry_matches_the_reference_encoder() {
        let h = letter_mono_header(4900, 6400);
        assert_eq!(h.mono_row_bytes(), 624);
        assert_eq!(h.jbig_width(), 4992);
        assert_eq!(h.payload_len().unwrap(), 31_360_000);
    }

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

    fn synthetic_tone_table() -> Vec<u8> {
        // Levels spell (grey + row + column) % 4, so the indexing and the row phase both show
        // up in the result.
        let mut t = vec![0u8; TONE_TABLE_LEN];
        for grey in 0..256usize {
            for r in 0..TONE_CELL {
                for c in 0..TONE_CELL {
                    let level = ((grey + r + c) % 4) as u8;
                    t[grey * TONE_CELL * TONE_ROW_BYTES + r * TONE_ROW_BYTES + c / 4] |=
                        level << (6 - 2 * (c % 4));
                }
            }
        }
        t
    }

    #[test]
    fn tone_level_follows_the_drivers_indexing() {
        let t = synthetic_tone_table();
        for (x, y) in [(0, 0), (3, 4), (7, 5), (11, 11), (12, 0), (13, 25)] {
            let row = (TONE_CELL - y % TONE_CELL) % TONE_CELL;
            let want = ((17 + row + x % TONE_CELL) % 4) as u8;
            assert_eq!(tone_level(&t, 17, x, y), want, "at ({x}, {y})");
        }
        // The cell repeats every twelve pixels across and every twelve lines down.
        assert_eq!(tone_level(&t, 17, 3, 4), tone_level(&t, 17, 15, 16));
    }

    #[test]
    fn two_bit_planes_carry_the_levels_bits() {
        let t = synthetic_tone_table();
        let mut h = letter_mono_header(8, 2);
        h.cups_bytes_per_line = 8;
        let px = [
            0u8, 5, 10, 20, 40, 60, 80, 100, 120, 140, 160, 180, 200, 220, 240, 255,
        ];
        let planes = to_two_bit_planes(&h, &px, &t).unwrap();
        assert_eq!(planes[0].len(), 32);
        assert_eq!(planes[1].len(), 32);
        for (y, row) in px.chunks(8).enumerate() {
            for (x, grey) in row.iter().enumerate() {
                let level = tone_level(&t, *grey, x, y);
                let bit = |p: usize| (planes[p][y * 16 + (x >> 3)] >> (7 - (x & 7))) & 1;
                assert_eq!(bit(0), level >> 1, "plane 0 at ({x}, {y})");
                assert_eq!(bit(1), level & 1, "plane 1 at ({x}, {y})");
            }
            assert!(
                planes[0][y * 16 + 1..(y + 1) * 16]
                    .iter()
                    .all(|b| *b == 0xFF),
                "row padding is the lightest level, so both planes are set there"
            );
            assert!(planes[1][y * 16 + 1..(y + 1) * 16]
                .iter()
                .all(|b| *b == 0xFF));
        }
    }

    #[test]
    fn a_tone_table_must_be_whole() {
        let h = letter_mono_header(8, 1);
        let err = to_two_bit_planes(&h, &[0u8; 8], &vec![0u8; TONE_TABLE_LEN - 1]).unwrap_err();
        assert!(matches!(err, RasterError::ToneTableShort { .. }), "{err}");
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
    fn rejects_a_page_that_would_exceed_the_payload_limit() {
        let h = header(8, 2, u32::MAX, 8, "Letter");
        let s = stream_pages(&[(&h, &[])]);
        let mut r = RasterReader::new(&s);
        assert!(matches!(
            r.next_page().unwrap_err(),
            RasterError::PayloadTooLarge { .. } | RasterError::GeometryOverflow
        ));
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
