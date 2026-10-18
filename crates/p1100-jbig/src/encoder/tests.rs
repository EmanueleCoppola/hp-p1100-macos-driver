//! The golden vectors and the geometry checks for the stripe encoder above them. They live in their own file so the encoder stays a page or two long.

use super::*;
use crate::qm::{MARKER_ESC, MARKER_SDNORM};

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn encode(width: u32, height: u32, bitmap: &[u8]) -> Vec<u8> {
    JbigEncoder::new(width, height).encode(bitmap).unwrap()
}

fn encode_with(width: u32, height: u32, options: u8, bitmap: &[u8]) -> Vec<u8> {
    JbigEncoder::with_options(width, height, DEFAULT_L0, DEFAULT_ORDER, options)
        .encode(bitmap)
        .unwrap()
}

#[test]
fn bih_layout_matches_the_standard() {
    let e = JbigEncoder::new(4992, 6400);
    let b = e.bih();
    assert_eq!(b.len(), 20);
    assert_eq!(b[0], 0); // DL
    assert_eq!(b[1], 0); // D, a single layer
    assert_eq!(b[2], 1); // one plane
    assert_eq!(&b[4..8], &4992u32.to_be_bytes());
    assert_eq!(&b[8..12], &6400u32.to_be_bytes());
    // L0 is four bytes, not one: the easiest field to get wrong
    assert_eq!(&b[12..16], &128u32.to_be_bytes());
    assert_eq!(b[16], 0); // MX
    assert_eq!(b[17], 0); // MY
    assert_eq!(b[18], 3); // ILEAVE | SMID
    assert_eq!(b[19], 0x5C); // DPON | TPBON | TPDON | LRLTWO
}

#[test]
fn stripe_count_follows_l0() {
    assert_eq!(JbigEncoder::new(8, 128).stripes(), 1);
    assert_eq!(JbigEncoder::new(8, 129).stripes(), 2);
    assert_eq!(JbigEncoder::new(8, 6400).stripes(), 50);
    assert_eq!(JbigEncoder::new(8, 1).stripes(), 1);
}

/// 8x2: row 0 = 10101010, row 1 blank. Byte-exact against the reference encoder, both
/// for the configuration we ship and for options = 0: those two take different paths
/// through the template choice and the typical-prediction bit.
#[test]
fn golden_8x2() {
    let want = hex("000001000000000800000002000000800000035ce164ff02");
    assert_eq!(encode(8, 2, &[0xAA, 0x00]), want);
    let no_options = hex("0000010000000008000000020000008000000300be80ff02");
    assert_eq!(encode_with(8, 2, 0, &[0xAA, 0x00]), no_options);
}

#[test]
fn high_bih_option_bit_does_not_select_sdrst() {
    let normal = encode_with(8, 2, 0, &[0xAA, 0x00]);
    let high_bit = encode_with(8, 2, 0x80, &[0xAA, 0x00]);
    assert_eq!(high_bit, normal);
}

#[test]
fn extreme_geometry_returns_an_error_on_every_target() {
    assert!(matches!(
        JbigEncoder::new(u32::MAX, u32::MAX).encode(&[]),
        Err(JbigError::GeometryOverflow | JbigError::BitmapTooSmall { .. })
    ));
}

/// Two planes: the fixture the reference encoder writes for a PGM of
/// `0 1 2 3 0 1 2 3 / 3 3 0 0 1 1 2 2` with binary (not Gray-coded) words.
#[test]
fn golden_two_planes_8x2() {
    let want = hex("000002000000000800000002000000800000031cc17080ff02d0eaff02");
    // Plane 0 is the high bit of each sample, plane 1 the low bit.
    let plane0 = [0x33u8, 0xC3];
    let plane1 = [0x55u8, 0xCC];
    // Options 0x1c: TPDON | TPBON | DPON, without LRLTWO. That is the configuration
    // the reference encoder picks for a multi-plane image (it sets LRLTWO only for a
    // bi-level one), and it is what the golden above was captured with.
    let got = JbigEncoder::with_options(8, 2, DEFAULT_L0, DEFAULT_ORDER, 0x1C)
        .planes(2)
        .encode_planes(&[&plane0, &plane1])
        .unwrap();
    assert_eq!(got, want);
}

/// Two planes across a stripe boundary: 8x130 samples of (x + y) % 4, so 130 lines is a
/// full stripe plus a two-line one. That is the case a per-plane coder state gets wrong.
#[test]
fn golden_two_planes_two_stripes() {
    let want =
        hex("000002000000000800000082000000800000031cc14eea73e05580ff02d0ce1fba64ff02ff02ff02");
    let mut plane0 = vec![0u8; 130];
    let mut plane1 = vec![0u8; 130];
    for (y, (hi, lo)) in plane0.iter_mut().zip(plane1.iter_mut()).enumerate() {
        let mut h = 0u8;
        let mut l = 0u8;
        for x in 0..8u32 {
            let v = (x + y as u32) % 4;
            if (v >> 1) & 1 == 1 {
                h |= 0x80 >> x;
            }
            if v & 1 == 1 {
                l |= 0x80 >> x;
            }
        }
        *hi = h;
        *lo = l;
    }
    let got = JbigEncoder::with_options(8, 130, DEFAULT_L0, DEFAULT_ORDER, 0x1C)
        .planes(2)
        .encode_planes(&[&plane0, &plane1])
        .unwrap();
    assert_eq!(got, want);
}

/// The number of bitmaps has to match the number of planes the encoder was built for.
#[test]
fn plane_count_must_match() {
    let one = [0u8; 2];
    let err = JbigEncoder::new(8, 2)
        .planes(2)
        .encode_planes(&[&one])
        .unwrap_err();
    assert!(
        matches!(
            err,
            JbigError::PlaneCount {
                expected: 2,
                got: 1
            }
        ),
        "{err}"
    );
    let err = JbigEncoder::new(8, 2)
        .encode_planes(&[&one, &one])
        .unwrap_err();
    assert!(
        matches!(
            err,
            JbigError::PlaneCount {
                expected: 1,
                got: 2
            }
        ),
        "{err}"
    );
    assert_eq!(JbigEncoder::new(8, 2).plane_count(), 1);
    assert_eq!(JbigEncoder::new(8, 2).planes(2).plane_count(), 2);
}

/// 16x4 with a denser pattern.
#[test]
fn golden_16x4() {
    let want = hex("000001000000001000000004000000800000035cfaa872927a5193ff02");
    let bm = [0xF0, 0x0F, 0x0F, 0xF0, 0xFF, 0x00, 0x00, 0xFF];
    assert_eq!(encode(16, 4, &bm), want);
    let no_options = hex("0000010000000010000000040000008000000300fac68596842853ff02");
    assert_eq!(encode_with(16, 4, 0, &bm), no_options);
}

/// 800x600, bands of 17 rows alternating solid and blank: five stripes, so this covers
/// the stripe loop and the carried probability state between stripes.
#[test]
fn golden_800x600_five_stripes() {
    let want = hex(
        "000001000000032000000258000000800000035cfd34d1d9f4629a6729aa86261a03ff02\
         509c758ea77bc922ff009760ff02e7130a84b1336b40065e80ff025e439f22e33e63f2f99fc0ff02\
         121d3fe58c4f9cff02",
    );
    let stride = 100usize; // 800 / 8
    let mut bm = vec![0u8; stride * 600];
    for y in 0..600 {
        if (y / 17) % 2 == 0 {
            for b in &mut bm[y * stride..(y + 1) * stride] {
                *b = 0xFF;
            }
        }
    }
    assert_eq!(encode(800, 600, &bm), want);
}

/// 1x200 single column: a degenerate width, and 200 rows over two stripes.
#[test]
fn golden_1x200_single_column() {
    let want = hex(
        "0000010000000001000000c8000000800000035cf303d2e768d6db6db6db6db6db6db6db70ff02\
         6090db6db6db6db6dcff02",
    );
    let bm: Vec<u8> = (0..200)
        .map(|y| if y % 3 == 0 { 0x80 } else { 0x00 })
        .collect();
    assert_eq!(encode(1, 200, &bm), want);
}

#[test]
fn terminator_and_stuffing_are_present() {
    let out = encode(8, 2, &[0xAA, 0x00]);
    // SDNORM terminator closes the only stripe, and no zero padding survives it
    assert_eq!(&out[out.len() - 2..], &[MARKER_ESC, MARKER_SDNORM]);
    assert_eq!(out.len(), 20 + 4);
}

#[test]
fn rejects_bitmaps_that_are_too_small() {
    let mut e = JbigEncoder::new(16, 4);
    assert_eq!(
        e.encode(&[0u8; 4]).unwrap_err(),
        JbigError::BitmapTooSmall { need: 8, got: 4 }
    );
    let mut e = JbigEncoder::new(0, 4);
    assert_eq!(e.encode(&[]).unwrap_err(), JbigError::Empty);
}

#[test]
fn an_all_blank_page_stays_tiny() {
    let out = encode(4992, 6400, &vec![0u8; 624 * 6400]);
    // 50 stripes, each compressing to very little
    assert!(out.len() < 2000, "blank page took {} bytes", out.len());
    assert_eq!(JbigEncoder::new(4992, 6400).stripes(), 50);
}
