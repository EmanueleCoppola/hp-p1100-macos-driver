//! Building a job from an in-memory 1 bpp bitmap instead of a raster stream: the path the
//! generated test pages and the offline tooling take.

use p1100_protocol::PageGeometry;
use p1100_raster::{PageHeader, RasterError};

use crate::error::Error;
use crate::image::write_mono_image;
use crate::job::encode_job;
use crate::options::Options;

/// Encode a single-page job from an already 1 bpp bitmap, using the stride
/// [`p1100_raster::to_mono1`] produces for this width.
///
/// This is the entry point for generated test pages, which never come from a raster
/// stream at all, so the whole job pipeline can be exercised without CUPS.
pub fn encode_bitmap_job(
    bitmap: &[u8],
    width: u32,
    height: u32,
    resolution: u32,
    paper: u32,
    opts: &Options,
) -> Result<Vec<u8>, Error> {
    let header = bitmap_header(width, height, resolution)?;
    let expected = header.payload_len()?;
    if bitmap.len() != expected {
        return Err(Error::Unsupported(format!(
            "bitmap must contain exactly {expected} bytes, got {}",
            bitmap.len()
        )));
    }
    let jbig_width = p1100_raster::align128(width);
    let mut g = PageGeometry::mono(width, height, resolution);
    g.paper = paper;
    g.duplex = opts.duplex;
    g.econo_mode = opts.econo_mode;
    g.source = opts.source;
    g.ret = true;
    encode_job(opts, |enc| {
        enc.page_start(&g);
        write_mono_image(enc, jbig_width, height, bitmap, opts)?;
        enc.page_end(opts.duplex);
        Ok(1)
    })
}

/// Validate generated bitmap geometry before allocating pixels or writing a job.
pub fn bitmap_header(width: u32, height: u32, resolution: u32) -> Result<PageHeader, Error> {
    let aligned = width
        .checked_add(127)
        .ok_or(RasterError::GeometryOverflow)?
        & !127;
    let header = PageHeader {
        cups_width: width,
        cups_height: height,
        hw_resolution: [resolution, resolution],
        cups_bits_per_color: 1,
        cups_bits_per_pixel: 1,
        cups_color_space: p1100_raster::color_space::K,
        cups_bytes_per_line: aligned / 8,
        ..PageHeader::default()
    };
    p1100_raster::validate_page_header(&header)?;
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::ImageEncoding;
    use crate::test_support::item_value;
    use p1100_protocol::{decode, paper, source, strip_pjl_framing};

    #[test]
    fn bitmap_jobs_validate_geometry_and_length_on_both_encodings() {
        for encoding in [ImageEncoding::Raw, ImageEncoding::Jbig] {
            let opts = Options {
                encoding,
                source: source::MANUAL,
                ..Options::default()
            };
            for (width, height, dpi, length) in [
                (0, 1, 600, 16),
                (u32::MAX, 1, 600, 16),
                (8, 0, 600, 0),
                (8, 1, 0, 16),
                (8, 1, 600, 15),
                (8, 1, 600, 17),
                (128, u32::MAX, 600, 16),
            ] {
                assert!(encode_bitmap_job(
                    &vec![0; length],
                    width,
                    height,
                    dpi,
                    paper::LETTER,
                    &opts
                )
                .is_err());
            }
            let job = encode_bitmap_job(&[0; 16], 8, 1, 600, paper::LETTER, &opts).unwrap();
            let chunks = decode(strip_pjl_framing(&job)).unwrap();
            assert_eq!(
                item_value(&chunks[1].items, p1100_protocol::item::DMDEFAULTSOURCE),
                source::MANUAL
            );
        }
    }
}
