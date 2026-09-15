//! Assemble a complete, printer-ready ZjStream job from a CUPS raster stream.
//!
//! This is what the CUPS filter wraps: `application/vnd.cups-raster` in, printer bytes
//! out. It is deliberately separate from the filter binary so it can be tested without
//! CUPS, and can later be re-hosted behind an IPP Printer Application without change
//! (see `docs/architecture.md`).

use p1005_protocol::{
    duplex as zduplex, media, op, paper, pjl_epilogue, pjl_prologue, source, Encoder, Items,
    JobOptions, PageGeometry, PjlSettings, Reserved, JBIG_BID_MAX, JBIG_BIH_LEN,
};
use p1005_raster::{to_mono1, PageHeader, RasterError, RasterStream, DEFAULT_THRESHOLD};

use p1005_jbig::JbigEncoder;

use std::io::{Read, Write};

/// PJL density default, matching the printer's normal setting.
pub const DEFAULT_DENSITY: u32 = 3;

/// How a page image is presented to the printer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEncoding {
    /// JBIG (T.82) compressed image, the format the printer expects. This is the default
    /// and what a real job uses.
    Jbig,
    /// `ZJT_RAW_IMAGE`: the uncompressed 1 bpp image, same bitmap the compressed path
    /// would feed to JBIG. The payload layout is a hypothesis that still has to be
    /// confirmed on hardware - see `docs/protocol.md`.
    Raw,
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Wrap the stream in the PJL prologue/epilogue.
    pub pjl: bool,
    pub density: u32,
    pub econo_mode: bool,
    /// `pjlJamRecovery`: true selects `AUTO`, false `OFF`.
    pub jam_recovery_auto: bool,
    /// Input tray, from the PPD's `InputSlot`.
    pub source: u32,
    /// Timestamp for the `JOBATTR` line, `YYYYMMDDHHMMSS`.
    pub timestamp: String,
    pub duplex: u32,
    pub encoding: ImageEncoding,
    pub threshold: u8,
    /// Override `DMPAPER`; `None` derives it from the raster's page size name.
    pub paper: Option<u32>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pjl: true,
            density: DEFAULT_DENSITY,
            econo_mode: false,
            jam_recovery_auto: false,
            source: source::ONLY_ONE,
            timestamp: "19700101000000".to_string(),
            duplex: zduplex::OFF,
            encoding: ImageEncoding::Jbig,
            threshold: DEFAULT_THRESHOLD,
            paper: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Raster(RasterError),
    Jbig(p1005_jbig::JbigError),
    /// Writing the job out failed.
    Io(String),
    /// The raster stream contained no pages.
    NoPages,
}

impl From<RasterError> for Error {
    fn from(e: RasterError) -> Self {
        Error::Raster(e)
    }
}

impl From<p1005_jbig::JbigError> for Error {
    fn from(e: p1005_jbig::JbigError) -> Self {
        Error::Jbig(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Raster(e) => write!(f, "{e}"),
            Error::Jbig(e) => write!(f, "compressing the page image: {e}"),
            Error::Io(e) => write!(f, "writing the job: {e}"),
            Error::NoPages => write!(f, "the raster stream contained no pages"),
        }
    }
}

impl std::error::Error for Error {}

/// Map a CUPS page size name to a `DMPAPER` code.
pub fn paper_code(page_size_name: &str) -> Option<u32> {
    let name = page_size_name.trim();
    let code = if name.eq_ignore_ascii_case("letter") {
        paper::LETTER
    } else if name.eq_ignore_ascii_case("legal") {
        paper::LEGAL
    } else if name.eq_ignore_ascii_case("executive") {
        paper::EXECUTIVE
    } else if name.eq_ignore_ascii_case("a3") {
        paper::A3
    } else if name.eq_ignore_ascii_case("a4") {
        paper::A4
    } else if name.eq_ignore_ascii_case("a5") {
        paper::A5
    } else if name.eq_ignore_ascii_case("b5") {
        paper::B5
    } else if name.eq_ignore_ascii_case("env dl") || name.eq_ignore_ascii_case("dl") {
        paper::ENV_DL
    } else {
        return None;
    };
    Some(code)
}

fn is_true(value: &str) -> bool {
    matches!(value, "true" | "True" | "1" | "on" | "On")
}

/// Apply a CUPS options string to the job options.
///
/// The names here are the ones this printer's PPD actually declares, taken from
/// `/etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd`:
///
/// | PPD option | values | maps to |
/// |---|---|---|
/// | `Duplex` | `None`, `DuplexNoTumble`, `DuplexTumble` | `ZJI_DMDUPLEX` |
/// | `InputSlot` | `Manual` | `ZJI_DMDEFAULTSOURCE` |
/// | `pjlDensity` | 1..5 | the PJL `DENSITY` line |
/// | `EconoMode` | `True` / `False` | the PJL `ECONOMODE` line and `ZJI_ECONOMODE` |
/// | `pjlJamRecovery` | `True` / `False` | the PJL `JAMRECOVERY` line |
///
/// CUPS passes those PPD names through to the filter, so handling only the IPP `sides=`
/// spelling would silently ignore a two-sided request. Both spellings are accepted.
/// `MediaType` needs no mapping: the PPD's `cupsMediaType` value reaches us in the raster
/// header, which is the authoritative code for the medium that was selected.
///
/// Unrecognised keys and values are ignored rather than guessed at.
pub fn apply_ppd_options(opts: &mut Options, options: &str) {
    for token in options.split_whitespace() {
        let (key, value) = match token.split_once('=') {
            Some(kv) => kv,
            None => continue,
        };
        match key {
            // IPP spelling
            "sides" => match value {
                "two-sided-long-edge" => opts.duplex = zduplex::LONG_EDGE,
                "two-sided-short-edge" => opts.duplex = zduplex::SHORT_EDGE,
                "one-sided" => opts.duplex = zduplex::OFF,
                _ => {}
            },
            // PPD spelling
            "Duplex" => match value {
                "DuplexNoTumble" => opts.duplex = zduplex::LONG_EDGE,
                "DuplexTumble" => opts.duplex = zduplex::SHORT_EDGE,
                "None" => opts.duplex = zduplex::OFF,
                _ => {}
            },
            "InputSlot" => match value {
                "Manual" => opts.source = source::MANUAL,
                "Upper" | "OnlyOne" => opts.source = source::ONLY_ONE,
                "Lower" => opts.source = source::LOWER,
                "Middle" => opts.source = source::MIDDLE,
                "Envelope" => opts.source = source::ENVELOPE,
                "Auto" => opts.source = source::AUTO,
                _ => {}
            },
            "pjlDensity" => {
                if let Ok(v) = value.parse::<u32>() {
                    // the PPD offers 1..5; ignore anything outside that range
                    if (1..=5).contains(&v) {
                        opts.density = v;
                    }
                }
            }
            "EconoMode" => opts.econo_mode = is_true(value),
            "pjlJamRecovery" => opts.jam_recovery_auto = is_true(value),
            _ => {}
        }
    }
}

/// Page geometry for one raster page.
pub fn geometry(header: &PageHeader, opts: &Options) -> PageGeometry {
    let mut g = PageGeometry::mono(
        header.cups_width,
        header.cups_height,
        header.hw_resolution[0].max(1),
    );
    g.resolution_y = header.hw_resolution[1].max(1);
    g.copies = header.num_copies.max(1);
    g.paper = opts
        .paper
        .or_else(|| paper_code(&header.cups_page_size_name))
        .unwrap_or(paper::LETTER);
    g.duplex = opts.duplex;
    g.econo_mode = opts.econo_mode;
    g.source = opts.source;
    // The media type code the PPD put into the raster header is the authoritative one for
    // the medium that was actually selected, so it is forwarded rather than re-derived
    // from the option name. Falls back to standard when it is unset.
    g.media = if header.cups_media_type != 0 {
        header.cups_media_type
    } else {
        media::STANDARD
    };
    g.ret = true;
    g
}

/// Write the image chunks for one already-converted 1 bpp bitmap.
fn write_mono_image(
    enc: &mut Encoder,
    jbig_width: u32,
    jbig_height: u32,
    mono: &[u8],
    opts: &Options,
) -> Result<(), Error> {
    match opts.encoding {
        ImageEncoding::Jbig => {
            // The image is presented to JBIG at the 128-pixel-aligned width, which is
            // exactly the stride `to_mono1` produced; RASTER_X keeps the true width.
            let mut jbig = JbigEncoder::new(jbig_width, jbig_height);
            let bie = jbig.encode(mono)?;
            debug_assert!(bie.len() >= JBIG_BIH_LEN);
            let chunks: Vec<&[u8]> = bie[JBIG_BIH_LEN..].chunks(JBIG_BID_MAX).collect();
            enc.jbig_image(&bie[..JBIG_BIH_LEN], &chunks);
        }
        ImageEncoding::Raw => {
            enc.chunk(op::RAW_IMAGE, &Items::new(), mono, Reserved::Zero);
        }
    }
    // The image-end marker follows the image on both paths.
    enc.end_jbig(opts.duplex);
    Ok(())
}

fn write_image(
    enc: &mut Encoder,
    header: &PageHeader,
    pixels: &[u8],
    opts: &Options,
) -> Result<(), Error> {
    let mono = to_mono1(header, pixels, opts.threshold)?;
    write_mono_image(enc, header.jbig_width(), header.cups_height, &mono, opts)
}

/// The PJL settings a job takes from the options.
fn pjl_settings(opts: &Options) -> PjlSettings<'_> {
    PjlSettings {
        density: opts.density,
        econo_mode: opts.econo_mode,
        jam_recovery_auto: opts.jam_recovery_auto,
        timestamp: &opts.timestamp,
    }
}

/// Write everything that precedes the first page: the PJL prologue, the stream signature and
/// the document chunks. Shared by the buffered and the streaming encoder.
fn start_job(enc: &mut Encoder, opts: &Options) {
    if opts.pjl {
        enc.extend_raw(&pjl_prologue(&pjl_settings(opts)));
    }
    enc.stream_signature();
    enc.doc_start(&JobOptions {
        duplex: opts.duplex,
        ..JobOptions::default()
    });
}

/// Write everything that follows the last page.
fn finish_job(enc: &mut Encoder, opts: &Options) {
    enc.doc_end();
    if opts.pjl {
        enc.extend_raw(&pjl_epilogue());
    }
}

/// Hand whatever the encoder has accumulated to the writer.
fn flush_pending<W: Write>(
    enc: &mut Encoder,
    pending: &mut Vec<u8>,
    out: &mut W,
) -> Result<(), Error> {
    enc.drain_into(pending);
    out.write_all(pending)
        .map_err(|e| Error::Io(e.to_string()))?;
    pending.clear();
    Ok(())
}

/// Build a job around a caller-supplied sequence of pages.
///
/// Both the raster path and the test-page path go through here, so the PJL wrapper, the
/// stream signature and the document chunks exist in exactly one place.
fn encode_job<F>(opts: &Options, mut emit_pages: F) -> Result<Vec<u8>, Error>
where
    F: FnMut(&mut Encoder) -> Result<usize, Error>,
{
    let mut enc = Encoder::new();
    start_job(&mut enc, opts);
    let pages = emit_pages(&mut enc)?;
    if pages == 0 {
        return Err(Error::NoPages);
    }
    finish_job(&mut enc, opts);
    Ok(enc.into_bytes())
}

/// Encode a CUPS raster stream to a writer, one page at a time.
///
/// This is what the filter uses. Peak memory is one page of raster plus one page's compressed
/// image rather than the whole job: a fifty page Letter document at 600 dpi is over a
/// gigabyte of raster, and nothing about producing the job needs all of it at once. Returns
/// the number of pages written.
pub fn encode_raster_stream<R: Read, W: Write>(
    reader: R,
    out: &mut W,
    opts: &Options,
) -> Result<usize, Error> {
    let mut enc = Encoder::new();
    let mut pending: Vec<u8> = Vec::new();

    start_job(&mut enc, opts);
    flush_pending(&mut enc, &mut pending, out)?;

    let mut stream = RasterStream::new(reader);
    let mut pixels: Vec<u8> = Vec::new();
    let mut pages = 0usize;
    while let Some(header) = stream.next_page(&mut pixels)? {
        enc.page_start(&geometry(&header, opts));
        write_image(&mut enc, &header, &pixels, opts)?;
        enc.page_end(opts.duplex);
        flush_pending(&mut enc, &mut pending, out)?;
        pages += 1;
    }
    if pages == 0 {
        return Err(Error::NoPages);
    }
    finish_job(&mut enc, opts);
    flush_pending(&mut enc, &mut pending, out)?;
    out.flush().map_err(|e| Error::Io(e.to_string()))?;
    Ok(pages)
}

/// Encode a whole CUPS raster that is already in memory. Convenient for tests and tooling;
/// the filter uses [`encode_raster_stream`] so that it does not have to hold a job.
pub fn encode_raster_job(raster: &[u8], opts: &Options) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    encode_raster_stream(raster, &mut out, opts)?;
    Ok(out)
}

/// Encode a single-page job from an already 1 bpp bitmap, using the stride
/// [`p1005_raster::to_mono1`] produces for this width.
///
/// This is the entry point for generated test pages, which never come from a raster
/// stream at all, so the whole job pipeline can be exercised without CUPS.
pub fn encode_bitmap_job(
    bitmap: &[u8],
    width: u32,
    height: u32,
    resolution: u32,
    paper: u32,
    opts: &Options,
) -> Result<Vec<u8>, Error> {
    let jbig_width = p1005_raster::align128(width);
    let mut g = PageGeometry::mono(width, height, resolution.max(1));
    g.paper = paper;
    g.duplex = opts.duplex;
    g.econo_mode = opts.econo_mode;
    g.ret = true;
    encode_job(opts, |enc| {
        enc.page_start(&g);
        write_mono_image(enc, jbig_width, height, bitmap, opts)?;
        enc.page_end(opts.duplex);
        Ok(1)
    })
}

// ---------------------------------------------------------------------------
// Deterministic test pages
// ---------------------------------------------------------------------------

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
    let stride = (p1005_raster::align128(width) / 8) as usize;
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

/// Format a Unix timestamp as `YYYYMMDDHHMMSS` in UTC, for the PJL `JOBATTR` line.
///
/// Done by hand rather than with a date crate: the value is cosmetic, and this keeps the
/// dependency list empty. Returns the epoch string for values that overflow the calendar.
pub fn format_timestamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    // Civil-from-days, Hinnant's algorithm, with the epoch shifted to 0000-03-01.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    if !(0..=9999).contains(&y) {
        return "19700101000000".to_string();
    }
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        y,
        m,
        d,
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

#[cfg(test)]
mod timestamp_tests {
    use super::format_timestamp;

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_timestamp(0), "19700101000000");
        assert_eq!(format_timestamp(1), "19700101000001");
        assert_eq!(format_timestamp(86_400), "19700102000000");
        // 2000-03-01T00:00:00Z, just after a leap day.
        assert_eq!(format_timestamp(951_868_800), "20000301000000");
        // 2026-09-15T12:34:56Z
        assert_eq!(format_timestamp(1_789_475_696), "20260915123456");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p1005_protocol::{decode, op as pop, op_name, strip_pjl_framing, STREAM_SIGNATURE};
    use p1005_raster::{color_space, compression, encode_page_header, SYNC_RASTER3};

    fn mono_header(width: u32, height: u32, name: &str) -> PageHeader {
        PageHeader {
            media_class: String::new(),
            media_color: String::new(),
            media_type: "Paper".into(),
            output_type: String::new(),
            hw_resolution: [600, 600],
            margins: [12, 12],
            num_copies: 1,
            collate: 0,
            duplex: 0,
            orientation: 0,
            page_size: [612, 792],
            cups_width: width,
            cups_height: height,
            cups_media_type: 1,
            cups_bits_per_color: 8,
            cups_bits_per_pixel: 8,
            cups_bytes_per_line: width,
            cups_color_order: 0,
            cups_color_space: color_space::W,
            cups_compression: compression::NONE,
            cups_row_count: 0,
            cups_row_feed: 0,
            cups_row_step: 0,
            cups_num_colors: 1,
            cups_borderless_scaling: 1.0,
            cups_page_size: [612.0, 792.0],
            cups_imaging_bbox: [12.0, 12.0, 600.0, 780.0],
            cups_page_size_name: name.into(),
            cups_marker_type: String::new(),
            cups_rendering_intent: String::new(),
        }
    }

    /// White page with a black pixel at the top-left corner.
    fn raster(pages: &[(u32, u32, &str)]) -> Vec<u8> {
        let mut out = SYNC_RASTER3.to_ne_bytes().to_vec();
        for (w, h, name) in pages {
            let header = mono_header(*w, *h, name);
            out.extend_from_slice(&encode_page_header(&header));
            let mut px = vec![255u8; (*w * *h) as usize];
            px[0] = 0;
            out.extend_from_slice(&px);
        }
        out
    }

    fn zjs_body(job: &[u8]) -> &[u8] {
        let body = strip_pjl_framing(job);
        assert_eq!(&body[..4], STREAM_SIGNATURE);
        body
    }

    #[test]
    fn one_page_job_is_jbig_compressed_by_default() {
        let r = raster(&[(8, 2, "Letter")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
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
    }

    /// End to end: the compressed image in the job, reassembled from its chunks, must be
    /// exactly what the JBIG encoder produced for the same bitmap.
    #[test]
    fn the_jobs_compressed_image_reassembles_to_the_encoder_output() {
        let (w, h) = (8u32, 2u32);
        let r = raster(&[(w, h, "Letter")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();

        let mut assembled = Vec::new();
        for c in &chunks {
            if c.header.ty == pop::JBIG_BIH || c.header.ty == pop::JBIG_BID {
                assembled.extend_from_slice(&c.payload);
            }
        }

        // Rebuild the bitmap the same way the job does, so the stride the raster layer
        // produces is the one the JBIG layer consumes.
        let header = mono_header(w, h, "Letter");
        let mut px = vec![255u8; (w * h) as usize];
        px[0] = 0;
        let mono = p1005_raster::to_mono1(&header, &px, p1005_raster::DEFAULT_THRESHOLD).unwrap();
        let mut jbig = JbigEncoder::new(p1005_raster::align128(w), h);
        let expected = jbig.encode(&mono).unwrap();

        // The last BID chunk is padded so the chunk ends on a 4-byte boundary.
        assert!(assembled.len() >= expected.len());
        assert_eq!(&assembled[..expected.len()], expected.as_slice());
        assert_eq!(chunks[2].payload.len(), JBIG_BIH_LEN);
    }

    /// A mismatch here would silently shear every printed page.
    #[test]
    fn raster_stride_matches_the_jbig_line_length() {
        for w in [1u32, 7, 8, 63, 64, 4900, 4992, 5000] {
            let header = mono_header(w, 4, "Letter");
            let jbig = JbigEncoder::new(p1005_raster::align128(w), 4);
            assert_eq!(
                header.mono_row_bytes(),
                jbig.bytes_per_line(),
                "width {w}: raster stride and JBIG line length must agree"
            );
        }
    }

    // ------------------------------------------------ CUPS options from the real PPD

    fn with_options(s: &str) -> Options {
        let mut o = Options::default();
        apply_ppd_options(&mut o, s);
        o
    }

    #[test]
    fn duplex_is_taken_from_the_ppd_spelling_not_just_ipp() {
        // These are the exact value names in the installed P1100 PPD. Handling only
        // "sides=" would silently ignore a two-sided request on a PPD queue.
        assert_eq!(
            with_options("Duplex=DuplexNoTumble").duplex,
            zduplex::LONG_EDGE
        );
        assert_eq!(
            with_options("Duplex=DuplexTumble").duplex,
            zduplex::SHORT_EDGE
        );
        assert_eq!(with_options("Duplex=None").duplex, zduplex::OFF);
        // and the IPP spelling still works
        assert_eq!(
            with_options("sides=two-sided-short-edge").duplex,
            zduplex::SHORT_EDGE
        );
        // a full option string, as CUPS would send it
        let o = with_options("media=A4 InputSlot=Manual pjlDensity=4 EconoMode=True job-uuid=x");
        assert_eq!(o.source, source::MANUAL);
        assert_eq!(o.density, 4);
        assert!(o.econo_mode);
    }

    #[test]
    fn density_is_clamped_to_what_the_ppd_offers() {
        assert_eq!(with_options("pjlDensity=5").density, 5);
        assert_eq!(with_options("pjlDensity=1").density, 1);
        // out of range or unparseable values leave the default alone
        assert_eq!(with_options("pjlDensity=9").density, DEFAULT_DENSITY);
        assert_eq!(with_options("pjlDensity=0").density, DEFAULT_DENSITY);
        assert_eq!(with_options("pjlDensity=x").density, DEFAULT_DENSITY);
    }

    #[test]
    fn jam_recovery_and_econo_mode_are_boolean_options() {
        assert!(with_options("pjlJamRecovery=True").jam_recovery_auto);
        assert!(!with_options("pjlJamRecovery=False").jam_recovery_auto);
        assert!(with_options("EconoMode=True").econo_mode);
        assert!(!with_options("EconoMode=False").econo_mode);
        // the PPD's default is False, which is where we start
        assert!(!Options::default().econo_mode);
        assert!(!Options::default().jam_recovery_auto);
    }

    #[test]
    fn unknown_keys_and_values_are_ignored_rather_than_guessed() {
        let o = with_options("InputSlot=Sideways Duplex=DuplexSideways nonsense media=A4");
        assert_eq!(o.source, source::ONLY_ONE);
        assert_eq!(o.duplex, zduplex::OFF);
        assert_eq!(o.density, DEFAULT_DENSITY);
        // a bare flag with no '=' must not disturb anything
        let o = with_options("Collate");
        assert_eq!(o.source, source::ONLY_ONE);
    }

    #[test]
    fn the_rasters_media_type_is_forwarded_to_the_printer() {
        // Paper is 1 and Transparency is 2 in the PPD, and those codes reach us in the
        // raster header rather than through the option string.
        let mut header = mono_header(8, 2, "Letter");
        header.cups_media_type = 2;
        let g = geometry(&header, &Options::default());
        assert_eq!(g.media, 2);

        header.cups_media_type = 0;
        let g = geometry(&header, &Options::default());
        assert_eq!(
            g.media,
            media::STANDARD,
            "unset media type falls back to standard"
        );
    }

    #[test]
    fn ppd_options_reach_the_page_items_and_the_pjl() {
        let o = with_options("Duplex=DuplexNoTumble InputSlot=Manual EconoMode=True");
        let r = raster(&[(8, 2, "Letter")]);
        let job = encode_raster_job(&r, &o).unwrap();
        let text = String::from_utf8_lossy(&job).into_owned();
        assert!(text.contains("@PJL SET ECONOMODE=ON\n"));

        let chunks = decode(strip_pjl_framing(&job)).unwrap();
        let page = chunks
            .iter()
            .find(|c| c.header.ty == pop::START_PAGE)
            .unwrap();
        assert_eq!(item_value(&page.items, p1005_protocol::item::ECONOMODE), 1);
        assert_eq!(
            item_value(&page.items, p1005_protocol::item::DMDUPLEX),
            zduplex::LONG_EDGE
        );
        assert_eq!(
            item_value(&page.items, p1005_protocol::item::DMDEFAULTSOURCE),
            source::MANUAL
        );
        // duplexing also puts DMDUPLEX on the page-end chunk
        let end_page = chunks
            .iter()
            .find(|c| c.header.ty == pop::END_PAGE)
            .unwrap();
        assert_eq!(
            item_value(&end_page.items, p1005_protocol::item::DMDUPLEX),
            zduplex::LONG_EDGE
        );
    }

    // ---------------------------------------------------------------- test pages

    fn item_value(items: &[p1005_protocol::DecodedItem], tag: u16) -> u32 {
        let it = items.iter().find(|i| i.item == tag).expect("item present");
        u32::from_be_bytes([it.payload[0], it.payload[1], it.payload[2], it.payload[3]])
    }

    fn ink(bm: &[u8], stride: usize, x: usize, y: usize) -> bool {
        bm[y * stride + (x >> 3)] & (0x80 >> (x & 7)) != 0
    }

    fn ink_count(bm: &[u8]) -> usize {
        bm.iter().map(|b| b.count_ones() as usize).sum()
    }

    fn stride_for(width: u32) -> usize {
        (p1005_raster::align128(width) / 8) as usize
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
            item_value(&chunks[1].items, p1005_protocol::item::RASTER_X),
            1300
        );
        assert_eq!(
            item_value(&chunks[1].items, p1005_protocol::item::VIDEO_X),
            1300
        );
        assert_eq!(
            item_value(&chunks[1].items, p1005_protocol::item::RASTER_Y),
            1300
        );
        assert_eq!(
            item_value(&chunks[1].items, p1005_protocol::item::RESOLUTION_X),
            600
        );
        assert_eq!(
            item_value(&chunks[1].items, p1005_protocol::item::DMPAPER),
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
        let mut jbig = JbigEncoder::new(p1005_raster::align128(1300), 1300);
        let expected = jbig.encode(&bm).unwrap();
        assert_eq!(&assembled[..expected.len()], expected.as_slice());
    }

    #[test]
    fn every_page_gets_its_own_start_and_end_page() {
        let r = raster(&[(8, 2, "Letter"), (8, 2, "Letter"), (8, 2, "Letter")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
        assert_eq!(
            chunks
                .iter()
                .filter(|c| c.header.ty == pop::START_PAGE)
                .count(),
            3
        );
        assert_eq!(
            chunks
                .iter()
                .filter(|c| c.header.ty == pop::END_PAGE)
                .count(),
            3
        );
        assert_eq!(
            chunks
                .iter()
                .filter(|c| c.header.ty == pop::JBIG_BIH)
                .count(),
            3
        );
        assert_eq!(chunks[0].header.ty, pop::START_DOC);
        assert_eq!(chunks.last().unwrap().header.ty, pop::END_DOC);
    }

    #[test]
    fn raw_image_payload_is_the_aligned_bitmap() {
        let r = raster(&[(8, 2, "Letter")]);
        let opts = Options {
            encoding: ImageEncoding::Raw,
            ..Options::default()
        };
        let job = encode_raster_job(&r, &opts).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
        let raw = chunks
            .iter()
            .find(|c| c.header.ty == pop::RAW_IMAGE)
            .unwrap();
        // width 8 -> aligned to 128 px -> 16 bytes per row, 2 rows
        assert_eq!(raw.payload.len(), 32);
        // only the very first pixel was black, and ink bits are set
        assert_eq!(raw.payload[0], 0b1000_0000);
        assert!(raw.payload[1..].iter().all(|b| *b == 0));
    }

    #[test]
    fn pjl_framing_is_present_by_default_and_absent_on_request() {
        let r = raster(&[(8, 2, "Letter")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let text = String::from_utf8_lossy(&job);
        assert!(text.starts_with("\u{1b}%-12345X@PJL JOB\n"));
        assert!(text.contains("@PJL SET DENSITY=3\n"));
        assert!(text.ends_with("@PJL EOJ\n\u{1b}%-12345X"));
        assert!(
            job.starts_with(b"\x1b%-12345X"),
            "job must open with the PJL UEL"
        );

        let opts = Options {
            pjl: false,
            ..Options::default()
        };
        let bare = encode_raster_job(&r, &opts).unwrap();
        assert_eq!(&bare[..4], STREAM_SIGNATURE);
    }

    #[test]
    fn paper_code_follows_the_cups_page_size_name() {
        assert_eq!(paper_code("Letter"), Some(paper::LETTER));
        assert_eq!(paper_code("letter"), Some(paper::LETTER));
        assert_eq!(paper_code("A4"), Some(paper::A4));
        assert_eq!(paper_code("A3"), Some(paper::A3));
        assert_eq!(paper_code("Legal"), Some(paper::LEGAL));
        assert_eq!(paper_code("Executive"), Some(paper::EXECUTIVE));
        assert_eq!(paper_code("A5"), Some(paper::A5));
        assert_eq!(paper_code("Env DL"), Some(paper::ENV_DL));
        assert_eq!(paper_code("Custom.100x200"), None);
    }

    #[test]
    fn page_size_name_reaches_the_printer_as_dmpaper() {
        let r = raster(&[(8, 2, "A4")]);
        let job = encode_raster_job(&r, &Options::default()).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
        let page = chunks
            .iter()
            .find(|c| c.header.ty == pop::START_PAGE)
            .unwrap();
        let dmpaper = page
            .items
            .iter()
            .find(|i| i.item == p1005_protocol::item::DMPAPER)
            .unwrap();
        assert_eq!(
            u32::from_be_bytes([
                dmpaper.payload[0],
                dmpaper.payload[1],
                dmpaper.payload[2],
                dmpaper.payload[3]
            ]),
            paper::A4
        );
    }

    #[test]
    fn an_empty_stream_is_an_error_rather_than_a_silent_empty_job() {
        let only_sync = SYNC_RASTER3.to_ne_bytes().to_vec();
        assert_eq!(
            encode_raster_job(&only_sync, &Options::default()).unwrap_err(),
            Error::NoPages
        );
    }

    #[test]
    fn a_corrupt_raster_is_reported_not_guessed() {
        let mut r = raster(&[(8, 2, "Letter")]);
        r.truncate(r.len() - 4); // geometry no longer fits
        assert!(matches!(
            encode_raster_job(&r, &Options::default()).unwrap_err(),
            Error::Raster(RasterError::GeometryMismatch { .. })
        ));
    }

    #[test]
    fn duplex_adds_a_duplex_item_to_the_page_and_image_end_chunks() {
        let r = raster(&[(8, 2, "Letter")]);
        let opts = Options {
            duplex: zduplex::LONG_EDGE,
            ..Options::default()
        };
        let job = encode_raster_job(&r, &opts).unwrap();
        let chunks = decode(zjs_body(&job)).unwrap();
        for ty in [pop::END_JBIG, pop::END_PAGE] {
            let c = chunks.iter().find(|c| c.header.ty == ty).unwrap();
            assert_eq!(c.items.len(), 1, "{ty} should carry DMDUPLEX");
        }
    }
}
