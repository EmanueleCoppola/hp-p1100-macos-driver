//! Assemble a complete, printer-ready ZjStream job from a CUPS raster stream.
//!
//! This is what the CUPS filter wraps: ~~application/vnd.cups-raster~~ in, printer bytes
//! out. It is deliberately separate from the filter binary so it can be tested without
//! CUPS, and can later be re-hosted behind an IPP Printer Application without change
//! (see ~~docs/architecture.md~~).
//!
//! How the crate is laid out
//! -------------------------
//! The options are what the caller asked for: ~~options~~ holds them, ~~ppd~~ reads CUPS's
//! option string and the printer's PPD defaults into them, and ~~geometry~~ turns a page's
//! raster header into the items the printer receives.
//!
//! ~~image~~ presents one page image and ~~job~~ assembles the chunks around it, streaming the
//! job out a page at a time. ~~bitmap~~ and ~~pattern~~ are the path that builds a job from a
//! generated bitmap rather than a raster stream, and ~~timestamp~~ formats the clock reading
//! the PJL line carries.
//!
//! Everything those modules expose is re-exported here, so callers keep using
//! ~~p1100_job::encode_raster_stream~~ and friends.

mod bitmap;
mod error;
mod geometry;
mod image;
mod job;
mod options;
mod paper;
mod pattern;
mod ppd;
mod timestamp;

#[cfg(test)]
mod test_support;

pub use bitmap::{bitmap_header, encode_bitmap_job};
pub use error::Error;
pub use geometry::geometry;
pub use job::{encode_raster_job, encode_raster_stream};
pub use options::{ImageEncoding, Options, ToneTables, DEFAULT_DENSITY};
pub use paper::paper_code;
pub use pattern::{test_page_bitmap, TestPattern};
pub use ppd::{apply_ppd_defaults, apply_ppd_options};
pub use timestamp::format_timestamp;

/// The crate root is the API: the module layout below it is an internal
/// detail, which is what the re-exports above keep true. Everything the crate exported
/// before it was split into modules is imported from the root here, so dropping a
/// re-export, moving an item or renaming one stops this module compiling rather than
/// reaching downstream users.
#[cfg(test)]
mod api_surface_tests {
    #[allow(unused_imports)]
    use crate::{
        apply_ppd_defaults, apply_ppd_options, bitmap_header, encode_bitmap_job, encode_raster_job,
        encode_raster_stream, format_timestamp, geometry, paper_code, test_page_bitmap, Error,
        ImageEncoding, Options, TestPattern, ToneTables, DEFAULT_DENSITY,
    };

    #[test]
    fn the_public_api_is_reachable_from_the_crate_root() {}
}
