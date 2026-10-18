//! The byte-level shape of a ZjStream: header sizes, signatures and payload constants.
//!
//! Every other module refers to these rather than repeating the numbers, and
//! `docs/protocol.md` is the prose description of the same format.

/// Chunk header: `size`, `type`, `items`, `reserved`, `signature`.
pub const CHUNK_HEADER_LEN: usize = 16;
/// Item header: `size`, `item`, `type`, `param`.
pub const ITEM_HEADER_LEN: usize = 8;
/// A `ZJIT_UINT32` item: its header plus the 4-byte value.
pub const ITEM_UINT32_LEN: usize = 12;
/// The value every chunk header carries in its `signature` field.
pub const CHUNK_SIGNATURE: u16 = 0x5a5a;
/// The four bytes a stream begins with.
pub const STREAM_SIGNATURE: &[u8; 4] = b"JZJZ";
/// The JBIG Bi-level Image Header is always exactly 20 bytes.
pub const JBIG_BIH_LEN: usize = 20;
/// Cap used by the reference encoder for each JBIG data chunk.
pub const JBIG_BID_MAX: usize = 65536;

/// Zero bytes the reference implementation adds to the end of the final image-data chunk,
/// beyond rounding the chunk up to 4 bytes. It is the default of its `-X` option, and its
/// own help text describes the padding as working around what the printer expects.
pub const JBIG_BID_EXTRA_PAD: usize = 16;

/// How to fill a chunk header's `reserved` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reserved {
    /// Write 0. Used by the reference encoder for item-less chunks.
    Zero,
    /// Write the total byte length of the item records that follow.
    Auto,
}
