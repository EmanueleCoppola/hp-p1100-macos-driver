//! The deterministic bring-up pages: patterns drawn straight into a 1 bpp bitmap, which is
//! how the whole job pipeline is exercised without CUPS or a physical page.

/// The deterministic bring-up pages the plan called for. Rendered for 600 dpi page
/// geometry, which is what the printer's driver produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestPattern {
    Blank,
    Square,
    HLine,
    VLine,
    Text123,
    Gray,
    Checker,
    Border,
}

impl TestPattern {
    pub const ALL: [TestPattern; 8] = [
        TestPattern::Blank,
        TestPattern::Square,
        TestPattern::HLine,
        TestPattern::VLine,
        TestPattern::Text123,
        TestPattern::Gray,
        TestPattern::Checker,
        TestPattern::Border,
    ];

    pub fn name(self) -> &'static str {
        match self {
            TestPattern::Blank => "blank",
            TestPattern::Square => "square",
            TestPattern::HLine => "hline",
            TestPattern::VLine => "vline",
            TestPattern::Text123 => "text123",
            TestPattern::Gray => "gray",
            TestPattern::Checker => "checker",
            TestPattern::Border => "border",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        TestPattern::ALL.into_iter().find(|p| p.name() == name)
    }
}

fn set_pixel(bm: &mut [u8], stride: usize, w: i64, h: i64, x: i64, y: i64) {
    if x < 0 || y < 0 || x >= w || y >= h {
        return;
    }
    bm[y as usize * stride + (x as usize >> 3)] |= 0x80 >> (x as usize & 7);
}

/// Render a pattern as a 1 bpp bitmap with stride `align128(width) / 8`, set bit = ink.
pub fn test_page_bitmap(pattern: TestPattern, width: u32, height: u32) -> Vec<u8> {
    let stride = (p1100_raster::align128(width) / 8) as usize;
    let mut bm = vec![0u8; stride * height as usize];
    let (w, h) = (width as i64, height as i64);

    match pattern {
        TestPattern::Blank => {}
        TestPattern::Square => {
            // a one inch square, one inch in from the top-left corner
            for y in 600..1200 {
                for x in 600..1200 {
                    set_pixel(&mut bm, stride, w, h, x, y);
                }
            }
        }
        TestPattern::HLine => {
            for y in h / 2..h / 2 + 8 {
                for x in 0..w {
                    set_pixel(&mut bm, stride, w, h, x, y);
                }
            }
        }
        TestPattern::VLine => {
            for x in w / 2..w / 2 + 8 {
                for y in 0..h {
                    set_pixel(&mut bm, stride, w, h, x, y);
                }
            }
        }
        TestPattern::Border => {
            for x in 0..w {
                set_pixel(&mut bm, stride, w, h, x, 50);
                set_pixel(&mut bm, stride, w, h, x, h - 51);
            }
            for y in 0..h {
                set_pixel(&mut bm, stride, w, h, 50, y);
                set_pixel(&mut bm, stride, w, h, w - 51, y);
            }
        }
        TestPattern::Checker => {
            for y in 0..h {
                for x in 0..w {
                    if ((x >> 5) + (y >> 5)) % 2 == 0 {
                        set_pixel(&mut bm, stride, w, h, x, y);
                    }
                }
            }
        }
        TestPattern::Gray => {
            // Eight bands of increasing ink density (1/8 .. 8/8), enough to see how the
            // printer renders a ramp and where it crushes shading.
            let bands = 8i64;
            let band_h = h / bands;
            for b in 0..bands {
                let step = b + 1;
                for y in b * band_h..(((b + 1) * band_h).min(h)) {
                    for x in 0..w {
                        if x % 8 < step {
                            set_pixel(&mut bm, stride, w, h, x, y);
                        }
                    }
                }
            }
        }
        TestPattern::Text123 => {
            const GLYPHS: [(&str, [&str; 7]); 7] = [
                (
                    "T",
                    [
                        "11111", "00100", "00100", "00100", "00100", "00100", "00100",
                    ],
                ),
                (
                    "E",
                    [
                        "11111", "10000", "10000", "11110", "10000", "10000", "11111",
                    ],
                ),
                (
                    "S",
                    [
                        "01111", "10000", "10000", "01110", "00001", "00001", "11110",
                    ],
                ),
                (
                    "1",
                    [
                        "00100", "01100", "00100", "00100", "00100", "00100", "01110",
                    ],
                ),
                (
                    "2",
                    [
                        "01110", "10001", "00001", "00010", "00100", "01000", "11111",
                    ],
                ),
                (
                    "3",
                    [
                        "11110", "00001", "00001", "01110", "00001", "00001", "11110",
                    ],
                ),
                (
                    " ",
                    [
                        "00000", "00000", "00000", "00000", "00000", "00000", "00000",
                    ],
                ),
            ];
            let scale = 24i64;
            let (x0, y0) = (300i64, 300i64);
            let text = ["T", "E", "S", "T", " ", "1", "2", "3"];
            for (i, ch) in text.iter().enumerate() {
                let glyph = GLYPHS.iter().find(|(c, _)| c == ch).map(|(_, rows)| *rows);
                if let Some(rows) = glyph {
                    let gx0 = x0 + i as i64 * 6 * scale;
                    for (gy, row) in rows.iter().enumerate() {
                        for (gx, c) in row.chars().enumerate() {
                            if c == '1' {
                                for dy in 0..scale {
                                    for dx in 0..scale {
                                        set_pixel(
                                            &mut bm,
                                            stride,
                                            w,
                                            h,
                                            gx0 + gx as i64 * scale + dx,
                                            y0 + gy as i64 * scale + dy,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    bm
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitmap::encode_bitmap_job;
    use crate::options::Options;
    use crate::test_support::item_value;
    use p1100_jbig::JbigEncoder;
    use p1100_protocol::{decode, op as pop, op_name, paper, strip_pjl_framing};

    fn ink(bm: &[u8], stride: usize, x: usize, y: usize) -> bool {
        bm[y * stride + (x >> 3)] & (0x80 >> (x & 7)) != 0
    }

    fn ink_count(bm: &[u8]) -> usize {
        bm.iter().map(|b| b.count_ones() as usize).sum()
    }

    fn stride_for(width: u32) -> usize {
        (p1100_raster::align128(width) / 8) as usize
    }

    #[test]
    fn pattern_names_round_trip() {
        for p in TestPattern::ALL {
            assert_eq!(TestPattern::parse(p.name()), Some(p));
        }
        assert_eq!(TestPattern::parse("nope"), None);
    }

    #[test]
    fn every_pattern_fills_the_page_at_the_aligned_stride() {
        for p in TestPattern::ALL {
            let bm = test_page_bitmap(p, 800, 600);
            assert_eq!(bm.len(), stride_for(800) * 600, "pattern {}", p.name());
        }
    }

    #[test]
    fn blank_is_empty_and_the_square_is_one_inch_in() {
        let blank = test_page_bitmap(TestPattern::Blank, 1300, 1300);
        assert_eq!(ink_count(&blank), 0);

        let bm = test_page_bitmap(TestPattern::Square, 1300, 1300);
        let s = stride_for(1300);
        assert!(ink(&bm, s, 600, 600));
        assert!(ink(&bm, s, 1199, 1199));
        assert!(!ink(&bm, s, 599, 600));
        assert!(!ink(&bm, s, 1200, 1200));
        assert_eq!(ink_count(&bm), 600 * 600);
    }

    #[test]
    fn lines_and_border_land_where_expected() {
        let (w, h) = (1300u32, 600u32);
        let s = stride_for(w);

        let hline = test_page_bitmap(TestPattern::HLine, w, h);
        assert!(ink(&hline, s, 0, 300));
        assert!(ink(&hline, s, 1299, 307));
        assert!(!ink(&hline, s, 0, 299));
        assert!(!ink(&hline, s, 0, 308));
        assert_eq!(ink_count(&hline), 8 * 1300);

        let vline = test_page_bitmap(TestPattern::VLine, w, h);
        assert!(ink(&vline, s, 650, 0));
        assert!(ink(&vline, s, 657, 599));
        assert!(!ink(&vline, s, 649, 0));
        assert_eq!(ink_count(&vline), 8 * 600);

        let border = test_page_bitmap(TestPattern::Border, w, h);
        assert!(ink(&border, s, 0, 50));
        assert!(ink(&border, s, 50, 50));
        assert!(ink(&border, s, 1249, (h - 1) as usize));
        assert!(!ink(&border, s, 0, 0));
    }

    #[test]
    fn the_gray_ramp_gets_strictly_denser() {
        let (w, h) = (1300u32, 600u32);
        let bm = test_page_bitmap(TestPattern::Gray, w, h);
        let s = stride_for(w);
        let band = h as usize / 8;
        let counts: Vec<usize> = (0..8)
            .map(|b| ink_count(&bm[b * band * s..(b + 1) * band * s]))
            .collect();
        for i in 1..8 {
            assert!(
                counts[i] > counts[i - 1],
                "band {i} ({}) should be denser than band {} ({})",
                counts[i],
                i - 1,
                counts[i - 1]
            );
        }
        assert_eq!(counts[7], 1300 * band, "the last band should be solid");
    }

    #[test]
    fn the_text_pattern_draws_ink_and_stays_in_its_box() {
        let (w, h) = (2000u32, 800u32);
        let bm = test_page_bitmap(TestPattern::Text123, w, h);
        let s = stride_for(w);
        assert!(ink_count(&bm) > 1000);
        // the first glyph starts at (300, 300)
        assert!(ink(&bm, s, 310, 310));
        // nothing beyond the rendered text box
        assert!(!ink(&bm, s, 1600, 400));
        assert!(!ink(&bm, s, 400, 700));
    }
    #[test]
    fn a_test_page_becomes_a_well_formed_compressed_job() {
        let bm = test_page_bitmap(TestPattern::Square, 1300, 1300);
        let job =
            encode_bitmap_job(&bm, 1300, 1300, 600, paper::LETTER, &Options::default()).unwrap();
        let chunks = decode(strip_pjl_framing(&job)).unwrap();
        let ops: Vec<&str> = chunks.iter().map(|c| op_name(c.header.ty)).collect();
        assert_eq!(
            ops,
            vec![
                "START_DOC",
                "START_PAGE",
                "JBIG_BIH",
                "JBIG_BID",
                "END_JBIG",
                "END_PAGE",
                "END_DOC"
            ]
        );
        // RASTER_X and VIDEO_X keep the true width; only the JBIG image is aligned up
        assert_eq!(
            item_value(&chunks[1].items, p1100_protocol::item::RASTER_X),
            1300
        );
        assert_eq!(
            item_value(&chunks[1].items, p1100_protocol::item::VIDEO_X),
            1300
        );
        assert_eq!(
            item_value(&chunks[1].items, p1100_protocol::item::RASTER_Y),
            1300
        );
        assert_eq!(
            item_value(&chunks[1].items, p1100_protocol::item::RESOLUTION_X),
            600
        );
        assert_eq!(
            item_value(&chunks[1].items, p1100_protocol::item::DMPAPER),
            paper::LETTER
        );
        // the BIH announces the aligned width
        assert_eq!(&chunks[2].payload[4..8], &1408u32.to_be_bytes());

        // and it reassembles to exactly what the compressor produced
        let mut assembled = Vec::new();
        for c in &chunks {
            if c.header.ty == pop::JBIG_BIH || c.header.ty == pop::JBIG_BID {
                assembled.extend_from_slice(&c.payload);
            }
        }
        let mut jbig = JbigEncoder::new(p1100_raster::align128(1300), 1300);
        let expected = jbig.encode(&bm).unwrap();
        assert_eq!(&assembled[..expected.len()], expected.as_slice());
    }
}
