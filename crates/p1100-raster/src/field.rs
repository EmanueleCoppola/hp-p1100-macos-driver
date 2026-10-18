//! `offsetof()` values into CUPS's `cups_page_header2_t`.

pub const MEDIA_CLASS: usize = 0;
pub const MEDIA_COLOR: usize = 64;
pub const MEDIA_TYPE: usize = 128;
pub const OUTPUT_TYPE: usize = 192;
pub const ADVANCE_DISTANCE: usize = 256;
pub const COLLATE: usize = 264;
pub const DUPLEX: usize = 272;
pub const HW_RESOLUTION: usize = 276;
pub const IMAGING_BOUNDING_BOX: usize = 284;
pub const MARGINS: usize = 312;
pub const MANUAL_FEED: usize = 320;
pub const NUM_COPIES: usize = 340;
pub const ORIENTATION: usize = 344;
pub const PAGE_SIZE: usize = 352;
pub const CUPS_WIDTH: usize = 372;
pub const CUPS_HEIGHT: usize = 376;
pub const CUPS_MEDIA_TYPE: usize = 380;
pub const CUPS_BITS_PER_COLOR: usize = 384;
pub const CUPS_BITS_PER_PIXEL: usize = 388;
pub const CUPS_BYTES_PER_LINE: usize = 392;
pub const CUPS_COLOR_ORDER: usize = 396;
pub const CUPS_COLOR_SPACE: usize = 400;
pub const CUPS_COMPRESSION: usize = 404;
pub const CUPS_ROW_COUNT: usize = 408;
pub const CUPS_ROW_FEED: usize = 412;
pub const CUPS_ROW_STEP: usize = 416;
pub const CUPS_NUM_COLORS: usize = 420;
pub const CUPS_BORDERLESS_SCALING: usize = 424;
pub const CUPS_PAGE_SIZE: usize = 428;
pub const CUPS_IMAGING_BBOX: usize = 436;
pub const CUPS_INTEGER: usize = 452;
pub const CUPS_REAL: usize = 516;
pub const CUPS_STRING: usize = 580;
pub const CUPS_MARKER_TYPE: usize = 1604;
pub const CUPS_RENDERING_INTENT: usize = 1668;
pub const CUPS_PAGE_SIZE_NAME: usize = 1732;

#[cfg(test)]
mod tests {
    use crate::field;
    use crate::wire::PAGE_HEADER_LEN;

    #[test]
    fn header_size_matches_the_cups_definition() {
        // Last member is cupsPageSizeName[64], ending at 1732 + 64.
        assert_eq!(field::CUPS_PAGE_SIZE_NAME + 64, PAGE_HEADER_LEN);
        // Four 64-byte strings, then the rest of the version-2 members in order.
        assert_eq!(field::MEDIA_COLOR, field::MEDIA_CLASS + 64);
        assert_eq!(field::MEDIA_TYPE, field::MEDIA_COLOR + 64);
        assert_eq!(field::OUTPUT_TYPE, field::MEDIA_TYPE + 64);
        assert_eq!(field::ADVANCE_DISTANCE, field::OUTPUT_TYPE + 64);
        assert_eq!(field::CUPS_STRING, field::CUPS_REAL + 16 * 4);
        assert_eq!(field::CUPS_STRING + 16 * 64, field::CUPS_MARKER_TYPE);
        assert_eq!(field::CUPS_MARKER_TYPE + 64, field::CUPS_RENDERING_INTENT);
        assert_eq!(
            field::CUPS_RENDERING_INTENT + 64,
            field::CUPS_PAGE_SIZE_NAME
        );
        assert_eq!(field::CUPS_NUM_COLORS, field::CUPS_ROW_STEP + 4);
        assert_eq!(field::CUPS_BORDERLESS_SCALING, field::CUPS_NUM_COLORS + 4);
        assert_eq!(field::CUPS_PAGE_SIZE, field::CUPS_BORDERLESS_SCALING + 4);
        assert_eq!(field::CUPS_IMAGING_BBOX, field::CUPS_PAGE_SIZE + 8);
        assert_eq!(field::CUPS_INTEGER, field::CUPS_IMAGING_BBOX + 16);
        assert_eq!(field::CUPS_REAL, field::CUPS_INTEGER + 16 * 4);
        // After PageSize[2] come Separations, TraySwitch, Tumble, then cupsWidth.
        assert_eq!(field::CUPS_WIDTH, field::PAGE_SIZE + 20);
    }
}
