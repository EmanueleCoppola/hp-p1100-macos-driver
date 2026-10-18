//! Zenographics ZjStream (ZJS) encoder for HP host-based LaserJets.
//!
//! Target family: the HP LaserJet Professional P1100 series, which HP's own driver describes
//! with one `hp1100` PPD for the whole series. The P1102 is the model this implementation
//! was reverse-engineered and verified against.
//! Format documentation: `docs/protocol.md`.
//!
//! The encoder is pure: it writes into an in-memory buffer and never touches a
//! device, so it can be tested without a printer attached.
//!
//! Wire summary
//! -------------
//! A stream is `b"JZJZ"` followed by a flat sequence of chunks. Each chunk is a
//! 16-byte big-endian header (`size`, `type`, `items`, `reserved`, `signature`)
//! followed by `items` item records. Each item is an 8-byte big-endian header
//! (`size`, `item`, `type`, `param`) followed by its payload.
//!
//! How the crate is laid out
//! -------------------------
//! The format's own sizes and signatures are in the private `wire` module. The vocabulary the
//! headers are written in is public: the opcodes in [`op`], the item tags in [`item`], the
//! item payload types in [`item_type`], and the device code tables in [`duplex`].
//! [`paper`], [`source`] and [`media`]. The writing side is `items` (an item block) and
//! `encoder` (the chunk stream); the reading side is `decoder`. What a caller wants printed is
//! described by `page` and `job`, and `pjl` is the framing the stream is wrapped in for the
//! printer.
//!
//! Every type and function of those private modules is re-exported here, so the layout is an
//! internal detail: callers keep using `p1100_protocol::Encoder` and friends.

pub mod duplex;
pub mod item;
pub mod item_type;
pub mod media;
pub mod op;
pub mod paper;
pub mod source;

mod decoder;
mod encoder;
mod items;
mod job;
mod page;
mod pjl;
mod wire;

pub use decoder::{decode, op_name, ChunkHeader, DecodeError, DecodedChunk, DecodedItem};
pub use encoder::Encoder;
pub use items::Items;
pub use job::JobOptions;
pub use page::PageGeometry;
pub use pjl::{pjl_epilogue, pjl_prologue, pjl_status_off, strip_pjl_framing, PjlSettings, UEL};
pub use wire::{
    Reserved, CHUNK_HEADER_LEN, CHUNK_SIGNATURE, ITEM_HEADER_LEN, ITEM_UINT32_LEN,
    JBIG_BID_EXTRA_PAD, JBIG_BID_MAX, JBIG_BIH_LEN, STREAM_SIGNATURE,
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
        decode, op_name, pjl_epilogue, pjl_prologue, pjl_status_off, strip_pjl_framing,
        ChunkHeader, DecodeError, DecodedChunk, DecodedItem, Encoder, Items, JobOptions,
        PageGeometry, PjlSettings, Reserved, CHUNK_HEADER_LEN, CHUNK_SIGNATURE, ITEM_HEADER_LEN,
        ITEM_UINT32_LEN, JBIG_BID_EXTRA_PAD, JBIG_BID_MAX, JBIG_BIH_LEN, STREAM_SIGNATURE, UEL,
    };

    #[allow(unused_imports)]
    use crate::duplex::{is_duplexing, LONG_EDGE, OFF, SHORT_EDGE};

    #[allow(unused_imports)]
    use crate::item::{
        DMCOLLATE, DMCOPIES, DMDEFAULTSOURCE, DMDUPLEX, DMMEDIATYPE, DMPAPER, ECONOMODE,
        MINOLTA_FILENAME, MINOLTA_USERNAME, NBIE, OFFSET_X, OFFSET_Y, PAGECOUNT, PLANE, QUANTITY,
        RASTER_X, RASTER_Y, RESOLUTION_X, RESOLUTION_Y, RET, VIDEO_BPP, VIDEO_X, VIDEO_Y,
    };

    #[allow(unused_imports)]
    use crate::item_type::{BYTELUT, INT32, STRING, UINT32};

    #[allow(unused_imports)]
    use crate::media::{GLOSSY, STANDARD, TRANSPARENCY};

    #[allow(unused_imports)]
    use crate::op::{
        END_DOC, END_JBIG, END_PAGE, END_PLANE, JBIG_BID, JBIG_BIH, RAW_IMAGE, SIGNATURE,
        START_DOC, START_PAGE, START_PLANE,
    };

    #[allow(unused_imports)]
    use crate::paper::{
        A3, A4, A5, A6, B5, CUSTOM, DOUBLE_POSTCARD, ENV_10, ENV_B5, ENV_C5, ENV_DL, ENV_MONARCH,
        EXECUTIVE, FANFOLD_GERMAN_LEGAL, K16_184X260, K16_195X270, K16_197X273, LEGAL, LETTER,
        POSTCARD,
    };

    #[allow(unused_imports)]
    use crate::source::{AUTO, ENVELOPE, LOWER, MANUAL, MIDDLE, ONLY_ONE};

    #[test]
    fn the_public_api_is_reachable_from_the_crate_root() {}
}
