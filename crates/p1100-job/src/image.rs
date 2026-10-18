//! Presenting one converted page image to the printer.
//!
//! Either as a JBIG (T.82) compressed image - one plane for the thresholded page, two for the
//! screened one - or as the uncompressed ~~ZJT_RAW_IMAGE~~ bitmap, and always followed by the
//! image-end marker.

use p1100_jbig::JbigEncoder;
use p1100_protocol::{op, Encoder, Items, Reserved, JBIG_BID_MAX, JBIG_BIH_LEN};
use p1100_raster::{to_mono1, to_two_bit_planes, PageHeader};

use crate::error::Error;
use crate::options::{ImageEncoding, Options};

/// Write the image chunks for one already-converted 1 bpp bitmap.
pub(crate) fn write_mono_image(
    enc: &mut Encoder,
    jbig_width: u32,
    jbig_height: u32,
    mono: &[u8],
    opts: &Options,
) -> Result<(), Error> {
    match opts.encoding {
        ImageEncoding::Jbig => {
            // The image is presented to JBIG at the 128-pixel-aligned width, which is
            // exactly the stride `to_mono1` produced; RASTER_X keeps the true width.
            let mut jbig = JbigEncoder::new(jbig_width, jbig_height);
            let bie = jbig.encode(mono)?;
            debug_assert!(bie.len() >= JBIG_BIH_LEN);
            let chunks: Vec<&[u8]> = bie[JBIG_BIH_LEN..].chunks(JBIG_BID_MAX).collect();
            enc.jbig_image(&bie[..JBIG_BIH_LEN], &chunks);
        }
        ImageEncoding::Raw => {
            enc.chunk(op::RAW_IMAGE, &Items::new(), mono, Reserved::Zero);
        }
    }
    // The image-end marker follows the image on both paths.
    enc.end_jbig(opts.duplex);
    Ok(())
}

pub(crate) fn write_image(
    enc: &mut Encoder,
    header: &PageHeader,
    pixels: &[u8],
    opts: &Options,
) -> Result<(), Error> {
    let Some(tables) = opts.tone_tables.as_ref() else {
        let mono = to_mono1(header, pixels, opts.threshold)?;
        return write_mono_image(enc, header.jbig_width(), header.cups_height, &mono, opts);
    };
    let table = tables.for_page(header);
    if opts.encoding != ImageEncoding::Jbig {
        return Err(Error::Unsupported(
            "the two-bit page is only produced as a JBIG image; the raw image layout for two \
             planes has not been established"
                .into(),
        ));
    }
    // Two bits per pixel, screened through HP's table: two planes, each one bit of the level
    // with the most significant plane first, and without LRLTWO, which is the configuration
    // the reference encoder uses for a multi-plane image and the one the two-plane sweep
    // checks. See docs/differences.md item 11.
    let planes = to_two_bit_planes(header, pixels, table)?;
    let refs: [&[u8]; 2] = [&planes[0], &planes[1]];
    let mut jbig = JbigEncoder::with_options(
        header.jbig_width(),
        header.cups_height,
        p1100_jbig::DEFAULT_L0,
        p1100_jbig::DEFAULT_ORDER,
        p1100_jbig::DEFAULT_OPTIONS & !p1100_jbig::OPT_LRLTWO,
    )
    .planes(2);
    let bie = jbig.encode_planes(&refs)?;
    debug_assert!(bie.len() >= JBIG_BIH_LEN);
    let chunks: Vec<&[u8]> = bie[JBIG_BIH_LEN..].chunks(JBIG_BID_MAX).collect();
    enc.jbig_image(&bie[..JBIG_BIH_LEN], &chunks);
    enc.end_jbig(opts.duplex);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode_raster_job;
    use crate::options::ToneTables;
    use crate::test_support::{compression_raster, table};
    use p1100_protocol::{decode, strip_pjl_framing, STREAM_SIGNATURE};

    /// And it reaches the page image: a page marked 401 comes out exactly as it would with the
    /// 400 dpi table in both slots, and differently from the same page marked for 600 dpi.
    #[test]
    fn the_marker_reaches_the_image() {
        let as_400 = Options {
            tone_tables: Some(ToneTables {
                at_400: table(0),
                at_600: table(3),
            }),
            ..Options::default()
        };
        let both_400 = Options {
            tone_tables: Some(ToneTables {
                at_400: table(0),
                at_600: table(0),
            }),
            ..Options::default()
        };
        let marked_400 = encode_raster_job(&compression_raster(401), &as_400).unwrap();
        assert_eq!(
            marked_400,
            encode_raster_job(&compression_raster(401), &both_400).unwrap()
        );
        assert_ne!(
            marked_400,
            encode_raster_job(&compression_raster(1), &as_400).unwrap()
        );

        // A sanity check on the fixture itself: the job is a real one.
        let body = strip_pjl_framing(&marked_400);
        assert_eq!(&body[..4], STREAM_SIGNATURE);
        assert!(!decode(body).unwrap().is_empty());
    }
}
