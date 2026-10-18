//! Reading pages out of a stream of unknown length, one at a time.

use std::io::Read;

use crate::error::RasterError;
use crate::header::parse_header;
use crate::page_header::PageHeader;
use crate::validate::validate_page_header;
use crate::wire::{PAGE_HEADER_LEN, SYNC_RASTER3};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::RasterReader;
    use crate::test_support::{header, stream_pages};

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
}
