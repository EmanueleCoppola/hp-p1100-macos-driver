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
    pub fn payload_len(&self) -> usize {
        self.cups_bytes_per_line as usize * self.cups_height as usize
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
    /// Degenerate geometry that cannot be trusted.
    BadGeometry {
        width: u32,
        height: u32,
        bytes_per_line: u32,
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
            RasterError::BadGeometry {
                width,
                height,
                bytes_per_line,
            } => write!(
                f,
                "implausible raster geometry: {width}x{height} px, {bytes_per_line} bytes per line"
            ),
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

        if header.cups_width == 0 || header.cups_height == 0 || header.cups_bytes_per_line == 0 {
            return Err(RasterError::BadGeometry {
                width: header.cups_width,
                height: header.cups_height,
                bytes_per_line: header.cups_bytes_per_line,
            });
        }
        let min_line =
            (header.cups_width as usize * header.cups_bits_per_pixel as usize).div_ceil(8);
        if (header.cups_bytes_per_line as usize) < min_line {
            return Err(RasterError::BadGeometry {
                width: header.cups_width,
                height: header.cups_height,
                bytes_per_line: header.cups_bytes_per_line,
            });
        }
        if !matches!(header.cups_bits_per_pixel, 1 | 2 | 4 | 8) {
            return Err(RasterError::UnsupportedDepth(header.cups_bits_per_pixel));
        }

        // `cups_compression` is deliberately NOT used to gate parsing. It is only ever
        // populated from the PostScript page device dictionary (cups/raster-interpret.c)
        // and libcups never consults it when reading pixels: page data is always
        // `cupsBytesPerLine * cupsHeight` raw bytes. Real cgpdftoraster output here carries
        // the value 401, which is not a CUPS compression code at all, so trusting it would
        // reject valid jobs. This byte-count check is the real validation: a genuinely
        // compressed stream would not match and we fail loudly on that instead.
        let payload_len = header.payload_len();
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

/// Convert one page of CUPS raster pixels into a 1-bit-per-pixel bitmap.
///
/// A set bit means ink, matching the reference encoder's dot counting. Rows are padded
/// to [`PageHeader::mono_row_bytes`].
pub fn to_mono1(header: &PageHeader, pixels: &[u8], threshold: u8) -> Result<Vec<u8>, RasterError> {
    let w = header.cups_width as usize;
    let h = header.cups_height as usize;
    let stride = header.mono_row_bytes();
    let src_stride = header.cups_bytes_per_line as usize;
    let mut out = vec![0u8; stride * h];
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
            let copy = src_stride.min(stride);
            for y in 0..h {
                out[y * stride..y * stride + copy]
                    .copy_from_slice(&pixels[y * src_stride..y * src_stride + copy]);
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

    fn letter_mono_header(width: u32, height: u32) -> PageHeader {
        PageHeader {
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
            cups_width: width,
            cups_height: height,
            cups_media_type: 1,
            cups_bits_per_color: 8,
            cups_bits_per_pixel: 8,
            cups_bytes_per_line: width,
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

    #[test]
    fn mono_geometry_matches_the_reference_encoder() {
        let h = letter_mono_header(4900, 6400);
        assert_eq!(h.mono_row_bytes(), 624);
        assert_eq!(h.jbig_width(), 4992);
        assert_eq!(h.payload_len(), 31_360_000);
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
        h.cups_bytes_per_line = 1;
        let mono = to_mono1(&h, &[0b1010_0000], DEFAULT_THRESHOLD).unwrap();
        assert_eq!(mono.len(), 16);
        assert_eq!(mono[0], 0b1010_0000);
        assert!(mono[1..].iter().all(|b| *b == 0), "row pad must be zeroed");
    }
}
