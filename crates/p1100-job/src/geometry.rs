//! Deriving one page's geometry from its raster page header and the job options.

use p1100_protocol::{media, paper, PageGeometry};
use p1100_raster::PageHeader;

use crate::options::Options;
use crate::paper::paper_code;

/// Page geometry for one raster page.
pub fn geometry(header: &PageHeader, opts: &Options) -> PageGeometry {
    let mut g = PageGeometry::mono(
        header.cups_width,
        header.cups_height,
        header.hw_resolution[0].max(1),
    );
    g.resolution_y = header.hw_resolution[1].max(1);
    g.copies = header.num_copies.max(1);
    g.paper = opts
        .paper
        .or_else(|| paper_code(&header.cups_page_size_name))
        .unwrap_or(paper::LETTER);
    g.duplex = opts.duplex;
    g.econo_mode = opts.econo_mode;
    g.source = opts.source;
    // Two-bit screening changes the page's depth: VIDEO_BPP, NBIE and RASTER_X follow from
    // `bits_per_pixel`, while VIDEO_X stays the logical pixel width.
    g.bits_per_pixel = if opts.tone_tables.is_some() { 2 } else { 1 };
    if g.bits_per_pixel == 2 {
        // HP reports the two-bit page at the rounded-up tone-cell height and takes its
        // vertical dpi from the cupsCompression marker (400 for the default PPD choice).
        g.height = header.two_bit_height();
        let marker = header.cups_compression & 0x3ffe;
        if marker != 0 {
            g.resolution_y = marker;
        }
    }
    // The media type code the PPD put into the raster header is the authoritative one for
    // the medium that was actually selected, so it is forwarded rather than re-derived
    // from the option name. Falls back to standard when it is unset.
    g.media = if header.cups_media_type != 0 {
        header.cups_media_type
    } else {
        media::STANDARD
    };
    g.ret = true;
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mono_header;

    #[test]
    fn the_rasters_media_type_is_forwarded_to_the_printer() {
        // Paper is 1 and Transparency is 2 in the PPD, and those codes reach us in the
        // raster header rather than through the option string.
        let mut header = mono_header(8, 2, "Letter");
        header.cups_media_type = 2;
        let g = geometry(&header, &Options::default());
        assert_eq!(g.media, 2);

        header.cups_media_type = 0;
        let g = geometry(&header, &Options::default());
        assert_eq!(
            g.media,
            media::STANDARD,
            "unset media type falls back to standard"
        );
    }
}
