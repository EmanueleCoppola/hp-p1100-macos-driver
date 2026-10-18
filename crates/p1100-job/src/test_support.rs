//! Fixtures shared by this crate's unit tests.
//!
//! Small on purpose: a raster stream, a page header, a job's ZjStream body and an item lookup,
//! so each test module says what it is testing rather than how to build a raster.

use p1100_protocol::{strip_pjl_framing, STREAM_SIGNATURE};
use p1100_raster::{encode_page_header, PageHeader, SYNC_RASTER3};

use crate::options::Options;
use crate::ppd::apply_ppd_options;

pub(crate) fn mono_header(width: u32, height: u32, name: &str) -> PageHeader {
    PageHeader {
        cups_width: width,
        cups_height: height,
        cups_bytes_per_line: width,
        cups_page_size_name: name.into(),
        ..PageHeader::default()
    }
}

/// White page with a black pixel at the top-left corner.
pub(crate) fn raster(pages: &[(u32, u32, &str)]) -> Vec<u8> {
    let mut out = SYNC_RASTER3.to_ne_bytes().to_vec();
    for (w, h, name) in pages {
        let header = mono_header(*w, *h, name);
        out.extend_from_slice(&encode_page_header(&header));
        let mut px = vec![255u8; (*w * *h) as usize];
        px[0] = 0;
        out.extend_from_slice(&px);
    }
    out
}

pub(crate) fn zjs_body(job: &[u8]) -> &[u8] {
    let body = strip_pjl_framing(job);
    assert_eq!(&body[..4], STREAM_SIGNATURE);
    body
}

pub(crate) fn with_options(s: &str) -> Options {
    let mut o = Options::default();
    apply_ppd_options(&mut o, s);
    o
}

pub(crate) fn item_value(items: &[p1100_protocol::DecodedItem], tag: u16) -> u32 {
    let it = items.iter().find(|i| i.item == tag).expect("item present");
    u32::from_be_bytes([it.payload[0], it.payload[1], it.payload[2], it.payload[3]])
}
pub(crate) fn compression_header(width: u32, height: u32, compression: u32) -> PageHeader {
    PageHeader {
        cups_width: width,
        cups_height: height,
        cups_bytes_per_line: width,
        cups_compression: compression,
        ..PageHeader::default()
    }
}

pub(crate) fn compression_raster(compression: u32) -> Vec<u8> {
    let mut out = SYNC_RASTER3.to_ne_bytes().to_vec();
    out.extend_from_slice(&encode_page_header(&compression_header(16, 2, compression)));
    out.extend((0..32u32).map(|i| (i * 8) as u8));
    out
}

pub(crate) fn table(level: u8) -> Vec<u8> {
    let mut t = vec![0u8; p1100_raster::TONE_TABLE_LEN];
    for byte in t.iter_mut() {
        *byte = level * 0x55;
    }
    t
}
