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

/// The two screening tables built into the filter when HP's own files are not installed.
///
/// They have the same shape as HP's: 256 entries of a 12x12 two-bit cell. The pattern is a
/// clustered-dot tile generated from four cluster centres on a 6x6 torus, repeated twice in
/// each direction. That keeps the driver independent of HP's binary while preserving the
/// four-level screening the printer expects; when HP's tables are present those are preferred
/// because they are the exact patterns its own driver used.
pub fn builtin_tone_tables() -> (Vec<u8>, Vec<u8>) {
    (builtin_tone_table(0), builtin_tone_table(1))
}

/// Build one 12x12 clustered-dot table. `variant` only moves the cluster centres, which gives
/// the 400 dpi and 600 dpi choices different screen phases.
pub fn builtin_tone_table(variant: u8) -> Vec<u8> {
    // Work in half-pixel coordinates so the centres can sit between pixels. The toroidal
    // distance makes the 6x6 tile wrap and therefore repeat cleanly in the 12x12 cell.
    let centres: [(i32, i32); 4] = if variant & 1 == 0 {
        [(3, 3), (9, 3), (3, 9), (9, 9)]
    } else {
        [(1, 1), (7, 1), (1, 7), (7, 7)]
    };
    let axis_distance = |a: i32, b: i32| {
        let d = (a - b).abs();
        let d = d.min(12 - d);
        d * d
    };
    let mut order: Vec<(i32, i32, i32)> = Vec::with_capacity(36);
    for y in 0..6i32 {
        for x in 0..6i32 {
            let (hx, hy) = (x * 2, y * 2);
            let distance = centres
                .iter()
                .map(|(cx, cy)| axis_distance(hx, *cx) + axis_distance(hy, *cy))
                .min()
                .unwrap_or(0);
            order.push((distance, x, y));
        }
    }
    order.sort_unstable();
    let mut rank = [[0u8; 6]; 6];
    for (r, (_, x, y)) in order.into_iter().enumerate() {
        // Spread the 36 tile ranks over the 144 cells. All four copies of a tile position
        // share a rank, which is what makes the 12x12 cell a 2x2 repeat of the 6x6 tile.
        rank[y as usize][x as usize] = (r * 4) as u8;
    }

    let mut table = vec![0u8; TONE_TABLE_LEN];
    for grey in 0..256usize {
        let base = grey * TONE_CELL * TONE_ROW_BYTES;
        for y in 0..TONE_CELL {
            for x in 0..TONE_CELL {
                let r = rank[y % 6][x % 6] as u32;
                // level = floor(grey/255*3 + (143-rank)/144), clamped to 0..3. The rational
                // form keeps every transition ordered without floating point.
                let numerator = 3 * grey as u32 * 144 + (143 - r) * 255;
                let level = (numerator / (255 * 144)).min(3) as u8;
                table[base + y * TONE_ROW_BYTES + x / 4] |= level << (6 - 2 * (x % 4));
            }
        }
    }
    table
}

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
    let expected = header.payload_len()?;
    if pixels.len() < expected {
        return Err(RasterError::ShortPixels {
            expected,
            got: pixels.len(),
        });
    }
    let w = header.cups_width as usize;
    let h = header.cups_height as usize;
    // HP's two-bit path aligns its packed row to 64 pixels, so each of the two bit planes
    // is 1/8 of that width. This is deliberately different from the 128-pixel alignment of
    // the one-bit JBIG image.
    let stride = header.two_bit_row_bytes();
    let bitmap_len = stride
        .checked_mul(header.cups_height as usize)
        .ok_or(RasterError::GeometryOverflow)?;
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
        assert_eq!(planes[0].len(), 16);
        assert_eq!(planes[1].len(), 16);
        for (y, row) in px.chunks(8).enumerate() {
            for (x, grey) in row.iter().enumerate() {
                let level = tone_level(&t, *grey, x, y);
                let bit = |p: usize| (planes[p][y * 8 + (x >> 3)] >> (7 - (x & 7))) & 1;
                assert_eq!(bit(0), level >> 1, "plane 0 at ({x}, {y})");
                assert_eq!(bit(1), level & 1, "plane 1 at ({x}, {y})");
            }
            assert!(
                planes[0][y * 8 + 1..(y + 1) * 8].iter().all(|b| *b == 0xFF),
                "row padding is the lightest level, so both planes are set there"
            );
            assert!(planes[1][y * 8 + 1..(y + 1) * 8].iter().all(|b| *b == 0xFF));
        }
    }
    #[test]
    fn a_tone_table_must_be_whole() {
        let h = letter_mono_header(8, 1);
        let err = to_two_bit_planes(&h, &[0u8; 8], &vec![0u8; TONE_TABLE_LEN - 1]).unwrap_err();
        assert!(matches!(err, RasterError::ToneTableShort { .. }), "{err}");
    }

    #[test]
    fn builtin_tables_have_hp_shape_and_grow_monotonically() {
        for variant in 0..2 {
            let table = builtin_tone_table(variant);
            assert_eq!(table.len(), TONE_TABLE_LEN);
            let cells = |grey: usize| -> Vec<Vec<u8>> {
                (0..TONE_CELL)
                    .map(|y| {
                        (0..TONE_CELL)
                            .map(|x| tone_level(&table, grey as u8, x, y))
                            .collect()
                    })
                    .collect()
            };
            assert!(cells(0).iter().all(|row| row.iter().all(|v| *v == 0)));
            assert!(cells(255).iter().all(|row| row.iter().all(|v| *v == 3)));

            let mut previous = 0usize;
            for grey in 0..=255 {
                let light = cells(grey).iter().flatten().filter(|v| **v == 3).count();
                assert!(light >= previous, "grey {grey} got darker");
                previous = light;
            }

            // The cell is a 2x2 repeat of a 6x6 tile, like HP's tables.
            let cell = cells(128);
            for y in 0..TONE_CELL {
                for x in 0..TONE_CELL {
                    assert_eq!(cell[y][x], cell[y % 6][x % 6]);
                }
            }
        }
    }
}
