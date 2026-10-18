//! HP's tone tables, and screening a grey page through one of them into two bit planes.

use crate::error::RasterError;
use crate::page_header::PageHeader;
use crate::validate::validate_page_header;
use crate::wire::MAX_BITMAP_PAYLOAD;

/// How many bytes a tone table takes: 256 grey levels, each a 12x12 cell of 2-bit values,
/// four cells per byte. `scripts/hp-tone-table.py --out` writes files of exactly this size,
/// read out of the installed HP driver.
pub const TONE_TABLE_LEN: usize = 256 * TONE_CELL * TONE_ROW_BYTES;

/// The cell's width and height, in pixels.
pub const TONE_CELL: usize = 12;

/// Bytes per cell row: twelve 2-bit values, padded to four bytes.
pub const TONE_ROW_BYTES: usize = 4;

/// One pixel's level, with HP's driver's indexing.
///
/// The table is indexed by the grey value first, then by the pixel's place in the cell: the
/// byte by its group of four columns and the 2-bit field inside it by its position in that
/// group, and the row by `(-y) % 12` - the driver's line counter counts down from the page
/// height rounded up to a multiple of 12, so the cell is walked upwards as the page is walked
/// downwards. A level of 0 is black and 3 is white, the way the tables are built.
pub fn tone_level(table: &[u8], grey: u8, x: usize, y: usize) -> u8 {
    let base = grey as usize * TONE_CELL * TONE_ROW_BYTES;
    let row = (TONE_CELL - y % TONE_CELL) % TONE_CELL;
    let byte = table[base + row * TONE_ROW_BYTES + (x % TONE_CELL) / 4];
    (byte >> (6 - 2 * (x % 4))) & 3
}

/// Screen one page of 8-bit grey into two bit planes, the way HP's driver does.
///
/// Each plane is one bit of the level, most significant plane first, packed with a set bit
/// meaning the level's bit is 1 - which is what the reference encoder writes when it splits a
/// two-bit PGM, so a two-bit page fed through it and through this function comes out the same.
/// Rows are padded to [`PageHeader::mono_row_bytes`] like the one-bit path's.
pub fn to_two_bit_planes(
    header: &PageHeader,
    pixels: &[u8],
    table: &[u8],
) -> Result<[Vec<u8>; 2], RasterError> {
    if table.len() < TONE_TABLE_LEN {
        return Err(RasterError::ToneTableShort {
            need: TONE_TABLE_LEN,
            got: table.len(),
        });
    }
    validate_page_header(header)?;
    if header.cups_bits_per_pixel != 8 {
        return Err(RasterError::UnsupportedDepth(header.cups_bits_per_pixel));
    }
    let bitmap_len = header.bitmap_len()?;
    let expected = header.payload_len()?;
    if pixels.len() < expected {
        return Err(RasterError::ShortPixels {
            expected,
            got: pixels.len(),
        });
    }
    let w = header.cups_width as usize;
    let h = header.cups_height as usize;
    let stride = header.mono_row_bytes();
    let src_stride = header.cups_bytes_per_line as usize;
    let total = bitmap_len
        .checked_mul(2)
        .ok_or(RasterError::GeometryOverflow)?;
    if total > MAX_BITMAP_PAYLOAD {
        return Err(RasterError::BitmapTooLarge {
            size: total,
            max: MAX_BITMAP_PAYLOAD,
        });
    }
    let mut planes = [vec![0u8; bitmap_len], vec![0u8; bitmap_len]];
    for y in 0..h {
        let src = &pixels[y * src_stride..];
        for x in 0..w {
            let level = tone_level(table, src[x], x, y);
            if level & 2 != 0 {
                planes[0][y * stride + (x >> 3)] |= 0x80 >> (x & 7);
            }
            if level & 1 != 0 {
                planes[1][y * stride + (x >> 3)] |= 0x80 >> (x & 7);
            }
        }
        // Everything past the page width takes the lightest level, which is what the one-bit
        // path's zeroed padding bits mean there: the printer draws the whole aligned width, so
        // a zeroed two-bit margin would print a black stripe down the right-hand side.
        for x in w..stride * 8 {
            planes[0][y * stride + (x >> 3)] |= 0x80 >> (x & 7);
            planes[1][y * stride + (x >> 3)] |= 0x80 >> (x & 7);
        }
    }
    Ok(planes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::letter_mono_header;

    fn synthetic_tone_table() -> Vec<u8> {
        // Levels spell (grey + row + column) % 4, so the indexing and the row phase both show
        // up in the result.
        let mut t = vec![0u8; TONE_TABLE_LEN];
        for grey in 0..256usize {
            for r in 0..TONE_CELL {
                for c in 0..TONE_CELL {
                    let level = ((grey + r + c) % 4) as u8;
                    t[grey * TONE_CELL * TONE_ROW_BYTES + r * TONE_ROW_BYTES + c / 4] |=
                        level << (6 - 2 * (c % 4));
                }
            }
        }
        t
    }
    #[test]
    fn tone_level_follows_the_drivers_indexing() {
        let t = synthetic_tone_table();
        for (x, y) in [(0, 0), (3, 4), (7, 5), (11, 11), (12, 0), (13, 25)] {
            let row = (TONE_CELL - y % TONE_CELL) % TONE_CELL;
            let want = ((17 + row + x % TONE_CELL) % 4) as u8;
            assert_eq!(tone_level(&t, 17, x, y), want, "at ({x}, {y})");
        }
        // The cell repeats every twelve pixels across and every twelve lines down.
        assert_eq!(tone_level(&t, 17, 3, 4), tone_level(&t, 17, 15, 16));
    }
    #[test]
    fn two_bit_planes_carry_the_levels_bits() {
        let t = synthetic_tone_table();
        let mut h = letter_mono_header(8, 2);
        h.cups_bytes_per_line = 8;
        let px = [
            0u8, 5, 10, 20, 40, 60, 80, 100, 120, 140, 160, 180, 200, 220, 240, 255,
        ];
        let planes = to_two_bit_planes(&h, &px, &t).unwrap();
        assert_eq!(planes[0].len(), 32);
        assert_eq!(planes[1].len(), 32);
        for (y, row) in px.chunks(8).enumerate() {
            for (x, grey) in row.iter().enumerate() {
                let level = tone_level(&t, *grey, x, y);
                let bit = |p: usize| (planes[p][y * 16 + (x >> 3)] >> (7 - (x & 7))) & 1;
                assert_eq!(bit(0), level >> 1, "plane 0 at ({x}, {y})");
                assert_eq!(bit(1), level & 1, "plane 1 at ({x}, {y})");
            }
            assert!(
                planes[0][y * 16 + 1..(y + 1) * 16]
                    .iter()
                    .all(|b| *b == 0xFF),
                "row padding is the lightest level, so both planes are set there"
            );
            assert!(planes[1][y * 16 + 1..(y + 1) * 16]
                .iter()
                .all(|b| *b == 0xFF));
        }
    }
    #[test]
    fn a_tone_table_must_be_whole() {
        let h = letter_mono_header(8, 1);
        let err = to_two_bit_planes(&h, &[0u8; 8], &vec![0u8; TONE_TABLE_LEN - 1]).unwrap_err();
        assert!(matches!(err, RasterError::ToneTableShort { .. }), "{err}");
    }
}
