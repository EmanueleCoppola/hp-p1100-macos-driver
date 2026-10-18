//! CUPS raster reader, and conversion to the 1-bit bitmap that the ZjStream encoder
//! hands to JBIG.
//!
//! This is the input side of our filter: CUPS invokes the printer's raster filter with
//! `application/vnd.cups-raster` on stdin, which is exactly where HP's `rastertozjs`
//! sits.
//!
//! Stream layout
//! -------------
//! ```text
//! u32 sync word                  once per stream, not per page
//! cups_page_header2_t            1796 bytes, per page
//! pixel data                     cupsBytesPerLine * cupsHeight bytes
//! cups_page_header2_t            next page, immediately after the previous pixels
//! ...
//! ```
//!
//! The sync word lives in `cupsRasterOpenIO`, which writes it when the stream is opened
//! and reads it back once (`cups/raster-stream.c`), so there is exactly one per stream.
//! `CUPS_RASTER_SYNC` is `0x52615333` (`"RaS3"`) and is the format `cgpdftoraster`
//! produces here; Apple also defines `CUPS_RASTER_SYNCapple` `0x554E4952` (`"UNIR"`).
//!
//! Field offsets are `offsetof()` into CUPS's current `cups_page_header2_t`, which is
//! why the struct has no `Magic`/`Version`/`HeaderSize` members: the sync word carries
//! that role and is stored out of band. Verified against the real header definition and
//! against captured output; see `docs/protocol.md`.

//! How the crate is laid out
//! -------------------------
//! The stream's own sizes and sync words are in `wire`, and the three tables CUPS defines -
//! the header offsets (`field`), the compression codes (`compression`) and the colour
//! spaces (`color_space`) - are public modules of their own. `page_header` owns the header
//! and the geometry derived from it, `header` maps it to and from its 1796-byte form, and
//! `validate` is the set of checks every consumer applies before a page goes on.
//!
//! Reading is split by what the caller already has: `reader` walks a stream already in
//! memory, `stream` pulls one page at a time out of a `Read`. `mono` converts a page's
//! pixels into the 1-bit bitmap the printer is fed, and `tone` screens them through HP's tone
//! table into the two-bit planes.
//!
//! Everything those modules expose is re-exported here, so callers keep using
//! `p1100_raster::to_mono1` and friends.

pub mod color_space;
pub mod compression;
pub mod field;

mod error;
mod header;
mod mono;
mod page_header;
mod reader;
mod stream;
mod tone;
mod validate;
mod wire;

#[cfg(test)]
mod test_support;

pub use error::RasterError;
pub use header::encode_page_header;
pub use mono::{to_mono1, DEFAULT_THRESHOLD};
pub use page_header::{align128, PageHeader};
pub use reader::RasterReader;
pub use stream::RasterStream;
pub use tone::{to_two_bit_planes, tone_level, TONE_CELL, TONE_ROW_BYTES, TONE_TABLE_LEN};
pub use validate::validate_page_header;
pub use wire::{
    MAX_BITMAP_PAYLOAD, MAX_PAGE_PAYLOAD, PAGE_HEADER_LEN, SYNC_APPLE, SYNC_RASTER1, SYNC_RASTER2,
    SYNC_RASTER3,
};

/// The crate root is the API: the module layout below it is an internal
/// detail, which is what the re-exports above keep true. Everything the crate exported
/// before it was split into modules is imported from the root here, so dropping a
/// re-export, moving an item or renaming one stops this module compiling rather than
/// reaching downstream users.
#[cfg(test)]
mod api_surface_tests {
    #[allow(unused_imports)]
    use crate::{
        align128, encode_page_header, to_mono1, to_two_bit_planes, tone_level,
        validate_page_header, PageHeader, RasterError, RasterReader, RasterStream,
        DEFAULT_THRESHOLD, MAX_BITMAP_PAYLOAD, MAX_PAGE_PAYLOAD, PAGE_HEADER_LEN, SYNC_APPLE,
        SYNC_RASTER1, SYNC_RASTER2, SYNC_RASTER3, TONE_CELL, TONE_ROW_BYTES, TONE_TABLE_LEN,
    };

    #[allow(unused_imports)]
    use crate::color_space::{K, RGB, SW, W};

    #[allow(unused_imports)]
    use crate::compression::{DEFLATE, GZIP, JPEG, LZW, NONE, PNG, RLE};

    #[allow(unused_imports)]
    use crate::field::{
        ADVANCE_DISTANCE, COLLATE, CUPS_BITS_PER_COLOR, CUPS_BITS_PER_PIXEL,
        CUPS_BORDERLESS_SCALING, CUPS_BYTES_PER_LINE, CUPS_COLOR_ORDER, CUPS_COLOR_SPACE,
        CUPS_COMPRESSION, CUPS_HEIGHT, CUPS_IMAGING_BBOX, CUPS_INTEGER, CUPS_MARKER_TYPE,
        CUPS_MEDIA_TYPE, CUPS_NUM_COLORS, CUPS_PAGE_SIZE, CUPS_PAGE_SIZE_NAME, CUPS_REAL,
        CUPS_RENDERING_INTENT, CUPS_ROW_COUNT, CUPS_ROW_FEED, CUPS_ROW_STEP, CUPS_STRING,
        CUPS_WIDTH, DUPLEX, HW_RESOLUTION, IMAGING_BOUNDING_BOX, MANUAL_FEED, MARGINS, MEDIA_CLASS,
        MEDIA_COLOR, MEDIA_TYPE, NUM_COPIES, ORIENTATION, OUTPUT_TYPE, PAGE_SIZE,
    };

    #[test]
    fn the_public_api_is_reachable_from_the_crate_root() {}
}
