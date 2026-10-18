//! Reading pages out of a raster stream already held in memory.

use crate::error::RasterError;
use crate::header::parse_header;
use crate::page_header::PageHeader;
use crate::validate::validate_page_header;
use crate::wire::{PAGE_HEADER_LEN, SYNC_RASTER3};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field;
    use crate::test_support::{header, stream_pages};

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
    fn rejects_a_page_that_would_exceed_the_payload_limit() {
        let h = header(8, 2, u32::MAX, 8, "Letter");
        let s = stream_pages(&[(&h, &[])]);
        let mut r = RasterReader::new(&s);
        assert!(matches!(
            r.next_page().unwrap_err(),
            RasterError::PayloadTooLarge { .. } | RasterError::GeometryOverflow
        ));
    }
}
