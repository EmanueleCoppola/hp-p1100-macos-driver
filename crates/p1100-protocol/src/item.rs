//! Item tags (`ZJ_ITEM`), the `item` field of an item record.

pub const PAGECOUNT: u16 = 0;
pub const DMCOLLATE: u16 = 1;
pub const DMDUPLEX: u16 = 2;
pub const DMPAPER: u16 = 3;
pub const DMCOPIES: u16 = 4;
pub const DMDEFAULTSOURCE: u16 = 5;
pub const DMMEDIATYPE: u16 = 6;
pub const NBIE: u16 = 7;
pub const RESOLUTION_X: u16 = 8;
pub const RESOLUTION_Y: u16 = 9;
pub const OFFSET_X: u16 = 10;
pub const OFFSET_Y: u16 = 11;
pub const RASTER_X: u16 = 12;
pub const RASTER_Y: u16 = 13;
pub const QUANTITY: u16 = 15;
pub const VIDEO_BPP: u16 = 16;
pub const VIDEO_X: u16 = 17;
pub const VIDEO_Y: u16 = 18;
pub const PLANE: u16 = 20;
pub const RET: u16 = 22;
pub const ECONOMODE: u16 = 23;

/// Minolta/OEM extension tags, kept because the reference encoder emits them for
/// user and file names. Not needed on the path this driver emits.
pub const MINOLTA_USERNAME: u16 = 0x810e;
pub const MINOLTA_FILENAME: u16 = 0x8115;
