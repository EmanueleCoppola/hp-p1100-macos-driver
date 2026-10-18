//! JBIG (ISO/IEC 11544 / ITU-T T.82) encoder.
//!
//! Written from the standard. Configuration emitted:
//!
//! ```text
//! DL = 0, D = 0 (a single resolution layer), planes = 1
//! order   = 3    (ILEAVE | SMID; irrelevant with one plane and one layer)
//! options = 0x5c (DPON | TPBON | TPDON | LRLTWO)
//! L0      = 128  lines per stripe
//! MX = MY = 0    no adaptive template movement
//! ```
//!
//! This is the configuration both working drivers use for this printer family, and it is
//! not an arbitrary choice: HP's own filter stores `0x5c` in the encoder constructor, and
//! the reference implementation reaches the same byte. The product is byte-identical to
//! the reference implementation for this configuration, which the oracle sweep and the
//! golden tests below check; see `docs/jbig.md`.
//!
//! Of the four bits, only TPBON changes the coded data here. TPDON and LRLTWO apply to the
//! low-resolution image of differential layers, which we do not emit, and DPON is a
//! decoder-side switch. LRLTWO is not quite inert even so: it selects which of the two
//! contexts T.82 reserves for the typical-prediction bit is used.
//!
//! The probability table is the normative one from T.82: a table of constants that every
//! implementation has to contain, not copied implementation code.
//!
//! Scope is deliberately narrow: one or more bit planes, one resolution layer, no adaptive template
//! movement, no differential layers. The pages this driver sends need nothing more.

//! How the crate is laid out
//! -------------------------
//! `tables` holds the standard's probability-state tables and `qm` the arithmetic coder
//! that reads them. `encoder` is the presentation layer on top: it computes the 20-byte
//! bi-level image header, splits the page into stripes, models the pixel neighbourhoods into
//! contexts and hands them to the coder. `error` says why an image could not be encoded.
//!
//! The items those modules expose are re-exported here, so callers keep using
//! `p1100_jbig::JbigEncoder` and friends.

mod encoder;
mod error;
mod qm;
mod tables;

pub use encoder::{
    JbigEncoder, DEFAULT_L0, DEFAULT_OPTIONS, DEFAULT_ORDER, OPT_DPON, OPT_LRLTWO, OPT_TPBON,
    OPT_TPDON,
};
pub use error::JbigError;
pub use qm::QmEncoder;

/// The crate root is the API: the module layout below it is an internal
/// detail, which is what the re-exports above keep true. Everything the crate exported
/// before it was split into modules is imported from the root here, so dropping a
/// re-export, moving an item or renaming one stops this module compiling rather than
/// reaching downstream users.
#[cfg(test)]
mod api_surface_tests {
    #[allow(unused_imports)]
    use crate::{
        JbigEncoder, JbigError, QmEncoder, DEFAULT_L0, DEFAULT_OPTIONS, DEFAULT_ORDER, OPT_DPON,
        OPT_LRLTWO, OPT_TPBON, OPT_TPDON,
    };

    #[test]
    fn the_public_api_is_reachable_from_the_crate_root() {}
}
