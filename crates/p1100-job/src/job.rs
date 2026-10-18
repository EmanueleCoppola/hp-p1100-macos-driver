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
mod tests;
