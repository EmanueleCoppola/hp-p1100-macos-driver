//! Fixtures shared by this crate's unit tests: a raw page header, a stream of pages built
//! from them, and a mono page header.

use crate::field;
use crate::page_header::PageHeader;
use crate::wire::{PAGE_HEADER_LEN, SYNC_RASTER3};

pub(crate) fn header(width: u32, height: u32, bpl: u32, bpp: u32, name: &str) -> Vec<u8> {
    let mut h = vec![0u8; PAGE_HEADER_LEN];
    h[field::MEDIA_TYPE..field::MEDIA_TYPE + 5].copy_from_slice(b"Paper");
    h[field::HW_RESOLUTION..field::HW_RESOLUTION + 4].copy_from_slice(&600u32.to_ne_bytes());
    h[field::HW_RESOLUTION + 4..field::HW_RESOLUTION + 8].copy_from_slice(&600u32.to_ne_bytes());
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
    h[field::CUPS_PAGE_SIZE..field::CUPS_PAGE_SIZE + 4].copy_from_slice(&612.0f32.to_ne_bytes());
    h[field::CUPS_PAGE_SIZE + 4..field::CUPS_PAGE_SIZE + 8]
        .copy_from_slice(&792.0f32.to_ne_bytes());
    h[field::CUPS_PAGE_SIZE_NAME..field::CUPS_PAGE_SIZE_NAME + name.len()]
        .copy_from_slice(name.as_bytes());
    h
}
pub(crate) fn stream_pages(pages: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut v = SYNC_RASTER3.to_ne_bytes().to_vec();
    for (h, p) in pages {
        v.extend_from_slice(h);
        v.extend_from_slice(p);
    }
    v
}
pub(crate) fn letter_mono_header(width: u32, height: u32) -> PageHeader {
    PageHeader {
        cups_width: width,
        cups_height: height,
        cups_bytes_per_line: width,
        ..PageHeader::default()
    }
}
