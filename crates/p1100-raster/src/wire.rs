//! The fixed sizes and sync words of a CUPS raster stream, and the resource caps this
//! reader enforces.
//!
//! The caps are host resource limits, not claims about what the printer accepts.

/// `CUPS_RASTER_SYNC`, `"RaS3"`.
pub const SYNC_RASTER3: u32 = 0x5261_5333;
/// `CUPS_RASTER_SYNCv2`, `"RaS2"` (also `CUPS_RASTER_SYNC_PWG`).
pub const SYNC_RASTER2: u32 = 0x5261_5332;
/// `CUPS_RASTER_SYNCv1`, `"RaSt"`.
pub const SYNC_RASTER1: u32 = 0x5261_5374;
/// `CUPS_RASTER_SYNCapple`, `"UNIR"`.
pub const SYNC_APPLE: u32 = 0x5545_4952;

/// `sizeof(cups_page_header2_t)` as measured against CUPS's current definition.
pub const PAGE_HEADER_LEN: usize = 1796;
/// Maximum raw pixel payload accepted for one page as a resource safety bound, not a printer limit.
pub const MAX_PAGE_PAYLOAD: usize = 128 * 1024 * 1024;
/// Maximum allocated bitmap size for one conversion (two planes count together). This is a
/// resource safety bound, not a printer limit.
pub const MAX_BITMAP_PAYLOAD: usize = 128 * 1024 * 1024;
