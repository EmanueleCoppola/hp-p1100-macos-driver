//! The 1796-byte on-the-wire form of a page header, both ways.

use crate::field;
use crate::page_header::PageHeader;
use crate::wire::PAGE_HEADER_LEN;

pub(crate) fn parse_header(h: &[u8]) -> PageHeader {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::RasterReader;
    use crate::test_support::{letter_mono_header, stream_pages};

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
}
