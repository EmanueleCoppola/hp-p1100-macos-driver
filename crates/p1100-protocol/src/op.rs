//! Chunk opcodes (`ZJ_TYPE`), the `type` field of a chunk header.

pub const START_DOC: u32 = 0;
pub const END_DOC: u32 = 1;
pub const START_PAGE: u32 = 2;
pub const END_PAGE: u32 = 3;
pub const JBIG_BIH: u32 = 4;
pub const JBIG_BID: u32 = 5;
pub const END_JBIG: u32 = 6;
pub const SIGNATURE: u32 = 7;
pub const RAW_IMAGE: u32 = 8;
pub const START_PLANE: u32 = 9;
pub const END_PLANE: u32 = 10;
