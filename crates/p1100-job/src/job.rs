//! The chunk sequence that makes up a job, and the streaming encoder that writes it out.
//!
//! The PJL wrapper, the stream signature and the document chunks exist in exactly one place,
//! whether the job is built from a raster stream or from a caller-supplied sequence of pages.

use std::io::{Read, Write};

use p1100_protocol::{pjl_epilogue, pjl_prologue, Encoder, JobOptions, PjlSettings};
use p1100_raster::RasterStream;

use crate::error::Error;
use crate::geometry::geometry;
use crate::image::write_image;
use crate::options::Options;

/// The PJL settings a job takes from the options.
fn pjl_settings(opts: &Options) -> PjlSettings<'_> {
    PjlSettings {
        density: opts.density,
        econo_mode: opts.econo_mode,
        jam_recovery_auto: opts.jam_recovery_auto,
        timestamp: &opts.timestamp,
    }
}

/// Write everything that precedes the first page: the PJL prologue, the stream signature and
/// the document chunks. Shared by the buffered and the streaming encoder.
fn start_job(enc: &mut Encoder, opts: &Options) {
    if opts.pjl {
        enc.extend_raw(&pjl_prologue(&pjl_settings(opts)));
    }
    enc.stream_signature();
    enc.doc_start(&JobOptions {
        duplex: opts.duplex,
        ..JobOptions::default()
    });
}

/// Write everything that follows the last page.
fn finish_job(enc: &mut Encoder, opts: &Options) {
    enc.doc_end();
    if opts.pjl {
        enc.extend_raw(&pjl_epilogue());
    }
}

/// Hand whatever the encoder has accumulated to the writer.
fn flush_pending<W: Write>(enc: &mut Encoder, out: &mut W) -> Result<(), Error> {
    out.write_all(enc.as_bytes())
        .map_err(|e| Error::Io(e.to_string()))?;
    enc.clear();
    Ok(())
}

/// Build a job around a caller-supplied sequence of pages.
///
/// Both the raster path and the test-page path go through here, so the PJL wrapper, the
/// stream signature and the document chunks exist in exactly one place.
pub(crate) fn encode_job<F>(opts: &Options, mut emit_pages: F) -> Result<Vec<u8>, Error>
where
    F: FnMut(&mut Encoder) -> Result<usize, Error>,
{
    let mut enc = Encoder::new();
    start_job(&mut enc, opts);
    let pages = emit_pages(&mut enc)?;
    if pages == 0 {
        return Err(Error::NoPages);
    }
    finish_job(&mut enc, opts);
    Ok(enc.into_bytes())
}

/// Encode a CUPS raster stream to a writer, one page at a time.
///
/// This is what the filter uses. Peak memory is one page of raster plus one page's compressed
/// image rather than the whole job: a fifty page Letter document at 600 dpi is over a
/// gigabyte of raster, and nothing about producing the job needs all of it at once. Returns
/// the number of pages written.
pub fn encode_raster_stream<R: Read, W: Write>(
    reader: R,
    out: &mut W,
    opts: &Options,
) -> Result<usize, Error> {
    let mut enc = Encoder::new();

    // Read the first page before writing anything. A stream that is not a raster at all - the
    // wrong file, or an upstream filter that died before its first page - then leaves the
    // printer with nothing, rather than a PJL prologue and an unfinished document to wait on.
    let mut stream = RasterStream::new(reader);
    let mut pixels: Vec<u8> = Vec::new();
    let mut next = stream.next_page(&mut pixels)?;
    if next.is_none() {
        return Err(Error::NoPages);
    }

    start_job(&mut enc, opts);
    // Keep the prologue in memory until the first image is encoded too. A valid raster
    // can still fail conversion (for example a missing/short screening table).

    let mut pages = 0usize;
    while let Some(header) = next.take() {
        enc.page_start(&geometry(&header, opts));
        write_image(&mut enc, &header, &pixels, opts)?;
        enc.page_end(opts.duplex);
        flush_pending(&mut enc, out)?;
        pages += 1;
        next = stream.next_page(&mut pixels)?;
    }
    finish_job(&mut enc, opts);
    flush_pending(&mut enc, out)?;
    out.flush().map_err(|e| Error::Io(e.to_string()))?;
    Ok(pages)
}

/// Encode a whole CUPS raster that is already in memory. Convenient for tests and tooling;
/// the filter uses [`encode_raster_stream`] so that it does not have to hold a job.
pub fn encode_raster_job(raster: &[u8], opts: &Options) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    encode_raster_stream(raster, &mut out, opts)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::{ImageEncoding, ToneTables};
    use crate::test_support::{item_value, mono_header, raster, with_options, zjs_body};
    use p1100_jbig::JbigEncoder;
    use p1100_protocol::{
        decode, duplex as zduplex, op as pop, op_name, paper, source, strip_pjl_framing, JBIG_BIH_LEN,
        STREAM_SIGNATURE,
    };
    use p1100_raster::{RasterError, SYNC_RASTER3};

    #[test]
    fn one_page_job_is_jbig_compressed_by_default() {
        let r = raster(&[(8, 2, "Letter")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
        let ops: Vec<&str> = chunks.iter().map(|c| op_name(c.header.ty)).collect();
        assert_eq!(
            ops,
            vec![
                "START_DOC",
                "START_PAGE",
                "JBIG_BIH",
                "JBIG_BID",
                "END_JBIG",
                "END_PAGE",
                "END_DOC"
            ]
        );
    }

    /// End to end: the compressed image in the job, reassembled from its chunks, must be
    /// exactly what the JBIG encoder produced for the same bitmap.
    #[test]
    fn the_jobs_compressed_image_reassembles_to_the_encoder_output() {
        let (w, h) = (8u32, 2u32);
        let r = raster(&[(w, h, "Letter")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();

        let mut assembled = Vec::new();
        for c in &chunks {
            if c.header.ty == pop::JBIG_BIH || c.header.ty == pop::JBIG_BID {
                assembled.extend_from_slice(&c.payload);
            }
        }

        // Rebuild the bitmap the same way the job does, so the stride the raster layer
        // produces is the one the JBIG layer consumes.
        let header = mono_header(w, h, "Letter");
        let mut px = vec![255u8; (w * h) as usize];
        px[0] = 0;
        let mono = p1100_raster::to_mono1(&header, &px, p1100_raster::DEFAULT_THRESHOLD).unwrap();
        let mut jbig = JbigEncoder::new(p1100_raster::align128(w), h);
        let expected = jbig.encode(&mono).unwrap();

        // The last BID chunk is padded so the chunk ends on a 4-byte boundary.
        assert!(assembled.len() >= expected.len());
        assert_eq!(&assembled[..expected.len()], expected.as_slice());
        assert_eq!(chunks[2].payload.len(), JBIG_BIH_LEN);
    }

    /// A mismatch here would silently shear every printed page.
    #[test]
    fn raster_stride_matches_the_jbig_line_length() {
        for w in [1u32, 7, 8, 63, 64, 4900, 4992, 5000] {
            let header = mono_header(w, 4, "Letter");
            let jbig = JbigEncoder::new(p1100_raster::align128(w), 4);
            assert_eq!(
                header.mono_row_bytes(),
                jbig.bytes_per_line(),
                "width {w}: raster stride and JBIG line length must agree"
            );
        }
    }

    // ------------------------------------------------ CUPS options from the real PPD

    #[test]
    fn ppd_options_reach_the_page_items_and_the_pjl() {
        let o = with_options("Duplex=DuplexNoTumble InputSlot=Manual EconoMode=True");
        let r = raster(&[(8, 2, "Letter")]);
        let job = encode_raster_job(&r, &o).unwrap();
        let text = String::from_utf8_lossy(&job).into_owned();
        assert!(text.contains("@PJL SET ECONOMODE=ON\n"));

        let chunks = decode(strip_pjl_framing(&job)).unwrap();
        let page = chunks
            .iter()
            .find(|c| c.header.ty == pop::START_PAGE)
            .unwrap();
        assert_eq!(item_value(&page.items, p1100_protocol::item::ECONOMODE), 1);
        assert_eq!(
            item_value(&page.items, p1100_protocol::item::DMDUPLEX),
            zduplex::LONG_EDGE
        );
        assert_eq!(
            item_value(&page.items, p1100_protocol::item::DMDEFAULTSOURCE),
            source::MANUAL
        );
        // duplexing also puts DMDUPLEX on the page-end chunk
        let end_page = chunks
            .iter()
            .find(|c| c.header.ty == pop::END_PAGE)
            .unwrap();
        assert_eq!(
            item_value(&end_page.items, p1100_protocol::item::DMDUPLEX),
            zduplex::LONG_EDGE
        );
    }

    // ---------------------------------------------------------------- test pages

    #[test]
    fn every_page_gets_its_own_start_and_end_page() {
        let r = raster(&[(8, 2, "Letter"), (8, 2, "Letter"), (8, 2, "Letter")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
        assert_eq!(
            chunks
                .iter()
                .filter(|c| c.header.ty == pop::START_PAGE)
                .count(),
            3
        );
        assert_eq!(
            chunks
                .iter()
                .filter(|c| c.header.ty == pop::END_PAGE)
                .count(),
            3
        );
        assert_eq!(
            chunks
                .iter()
                .filter(|c| c.header.ty == pop::JBIG_BIH)
                .count(),
            3
        );
        assert_eq!(chunks[0].header.ty, pop::START_DOC);
        assert_eq!(chunks.last().unwrap().header.ty, pop::END_DOC);
    }

    /// A document can mix page sizes - a Letter cover sheet and A4 body pages, say - and each
    /// page carries its own geometry, because CUPS sends a page header per page. Getting this
    /// wrong would put every page on the first page's paper, which on a printer that acts on
    /// the code would be felt rather than seen.
    #[test]
    fn a_document_can_mix_page_sizes() {
        let r = raster(&[(8, 2, "Letter"), (16, 4, "A4"), (8, 2, "B5JIS")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
        let pages: Vec<&Vec<p1100_protocol::DecodedItem>> = chunks
            .iter()
            .filter(|c| c.header.ty == pop::START_PAGE)
            .map(|c| &c.items)
            .collect();
        assert_eq!(pages.len(), 3);
        let codes: Vec<u32> = pages
            .iter()
            .map(|items| item_value(items, p1100_protocol::item::DMPAPER))
            .collect();
        assert_eq!(
            codes,
            vec![paper::LETTER, paper::A4, paper::B5],
            "each page keeps the code for its own size"
        );
        // The raster dimensions follow the same per-page header.
        let widths: Vec<u32> = pages
            .iter()
            .map(|items| item_value(items, p1100_protocol::item::RASTER_X))
            .collect();
        assert_eq!(widths, vec![8, 16, 8]);
        let heights: Vec<u32> = pages
            .iter()
            .map(|items| item_value(items, p1100_protocol::item::RASTER_Y))
            .collect();
        assert_eq!(heights, vec![2, 4, 2]);
    }

    #[test]
    fn raw_image_payload_is_the_aligned_bitmap() {
        let r = raster(&[(8, 2, "Letter")]);
        let opts = Options {
            encoding: ImageEncoding::Raw,
            ..Options::default()
        };
        let job = encode_raster_job(&r, &opts).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
        let raw = chunks
            .iter()
            .find(|c| c.header.ty == pop::RAW_IMAGE)
            .unwrap();
        // width 8 -> aligned to 128 px -> 16 bytes per row, 2 rows
        assert_eq!(raw.payload.len(), 32);
        // only the very first pixel was black, and ink bits are set
        assert_eq!(raw.payload[0], 0b1000_0000);
        assert!(raw.payload[1..].iter().all(|b| *b == 0));
    }

    #[test]
    fn pjl_framing_is_present_by_default_and_absent_on_request() {
        let r = raster(&[(8, 2, "Letter")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let text = String::from_utf8_lossy(&job);
        assert!(text.starts_with("\u{1b}%-12345X@PJL JOB\n"));
        assert!(text.contains("@PJL SET DENSITY=3\n"));
        assert!(text.ends_with("@PJL EOJ\n\u{1b}%-12345X"));
        assert!(
            job.starts_with(b"\x1b%-12345X"),
            "job must open with the PJL UEL"
        );

        let opts = Options {
            pjl: false,
            ..Options::default()
        };
        let bare = encode_raster_job(&r, &opts).unwrap();
        assert_eq!(&bare[..4], STREAM_SIGNATURE);
    }

    /// CUPS sends only the options a job named, so a filter has to read the PPD for the rest.
    /// HP's driver does exactly that; this is the same thing, for the options we map.
    #[test]
    fn page_size_name_reaches_the_printer_as_dmpaper() {
        let r = raster(&[(8, 2, "A4")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
        let page = chunks
            .iter()
            .find(|c| c.header.ty == pop::START_PAGE)
            .unwrap();
        let dmpaper = page
            .items
            .iter()
            .find(|i| i.item == p1100_protocol::item::DMPAPER)
            .unwrap();
        assert_eq!(
            u32::from_be_bytes([
                dmpaper.payload[0],
                dmpaper.payload[1],
                dmpaper.payload[2],
                dmpaper.payload[3]
            ]),
            paper::A4
        );
    }

    #[test]
    fn an_empty_stream_is_an_error_rather_than_a_silent_empty_job() {
        let only_sync = SYNC_RASTER3.to_ne_bytes().to_vec();
        assert_eq!(
            encode_raster_job(&only_sync, &Options::default()).unwrap_err(),
            Error::NoPages
        );
    }

    /// A stream that is not a raster at all must leave the writer untouched. The filter streams
    /// its output to the printer as it goes, so anything written before the input is validated
    /// is a PJL prologue and an unfinished document for the printer to wait on.
    #[test]
    fn a_stream_that_is_not_a_raster_writes_nothing() {
        let mut out: Vec<u8> = Vec::new();
        let err = encode_raster_stream(&b"not a raster at all"[..], &mut out, &Options::default())
            .unwrap_err();
        assert!(matches!(err, Error::Raster(_)), "{err:?}");
        assert!(
            out.is_empty(),
            "{} bytes were written before the error",
            out.len()
        );
    }

    #[test]
    fn first_page_conversion_failure_writes_nothing() {
        let r = raster(&[(8, 2, "Letter")]);
        for encoding in [ImageEncoding::Jbig, ImageEncoding::Raw] {
            let opts = Options {
                encoding,
                tone_tables: Some(ToneTables {
                    at_400: vec![],
                    at_600: vec![],
                }),
                ..Options::default()
            };
            let mut out = Vec::new();
            assert!(encode_raster_stream(&r[..], &mut out, &opts).is_err());
            assert!(
                out.is_empty(),
                "failed image left a partial job on the wire"
            );
        }
    }

    #[test]
    fn output_handles_short_writes_and_reports_failures() {
        struct Sink {
            bytes: Vec<u8>,
            fail_after: usize,
            interrupted: bool,
            zero: bool,
        }
        impl Write for Sink {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if !self.interrupted {
                    self.interrupted = true;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                if self.bytes.len() >= self.fail_after {
                    return if self.zero {
                        Ok(0)
                    } else {
                        Err(std::io::ErrorKind::BrokenPipe.into())
                    };
                }
                let n = bytes.len().min(3).min(self.fail_after - self.bytes.len());
                self.bytes.extend_from_slice(&bytes[..n]);
                Ok(n)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let r = raster(&[(8, 2, "Letter"), (16, 3, "A4")]);
        let opts = Options::default();
        let expected = encode_raster_job(&r, &opts).unwrap();
        let mut sink = Sink {
            bytes: vec![],
            fail_after: usize::MAX,
            interrupted: false,
            zero: false,
        };
        assert_eq!(encode_raster_stream(&r[..], &mut sink, &opts).unwrap(), 2);
        assert_eq!(sink.bytes, expected);
        for zero in [false, true] {
            let mut sink = Sink {
                bytes: vec![],
                fail_after: 40,
                interrupted: false,
                zero,
            };
            assert!(matches!(
                encode_raster_stream(&r[..], &mut sink, &opts),
                Err(Error::Io(_))
            ));
            assert_eq!(sink.bytes, expected[..40]);
        }
    }

    #[test]
    fn truncated_second_page_does_not_claim_a_complete_job() {
        let mut r = raster(&[(8, 2, "Letter"), (16, 3, "A4")]);
        r.pop();
        let mut out = Vec::new();
        assert!(encode_raster_stream(&r[..], &mut out, &Options::default()).is_err());
        let chunks = decode(strip_pjl_framing(&out)).unwrap();
        assert_eq!(
            chunks
                .iter()
                .filter(|c| c.header.ty == pop::START_PAGE)
                .count(),
            1
        );
        assert_eq!(chunks.last().unwrap().header.ty, pop::END_PAGE);
    }
    #[test]
    fn a_corrupt_raster_is_reported_not_guessed() {
        let mut r = raster(&[(8, 2, "Letter")]);
        r.truncate(r.len() - 4); // geometry no longer fits
        assert!(matches!(
            encode_raster_job(&r, &Options::default()).unwrap_err(),
            Error::Raster(RasterError::GeometryMismatch { .. })
        ));
    }

    #[test]
    fn duplex_adds_a_duplex_item_to_the_page_and_image_end_chunks() {
        let r = raster(&[(8, 2, "Letter")]);
        let opts = Options {
            duplex: zduplex::LONG_EDGE,
            ..Options::default()
        };
        let job = encode_raster_job(&r, &opts).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
        for ty in [pop::END_JBIG, pop::END_PAGE] {
            let c = chunks.iter().find(|c| c.header.ty == ty).unwrap();
            assert_eq!(c.items.len(), 1, "{ty} should carry DMDUPLEX");
        }
    }
}
