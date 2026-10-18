//! Producing a printable job: from a CUPS raster stream, or from a generated test page.

use std::fs;

use p1100_job::{encode_raster_job, Options as JobOpts, TestPattern};

use crate::read_file;

/// Measured 600 dpi page geometry: the raster's dimensions, which are the PPD's imageable area
/// with its 12 pt margins taken off rather than the paper's - for Legal 588 x 984 pt, 4900 x
/// 8200 px. scripts/bringup-compare.py holds this to what the filter derives from a real CUPS
/// raster, which is how the Legal height here was found to be the paper's 8100.
const TEST_PAGES: [(&str, u32, u32, u32); 3] = [
    ("letter", 4900, 6400, 1),
    ("a4", 4758, 6817, 9),
    ("legal", 4900, 8200, 5),
];
const DEFAULT_TEST_DPI: u32 = 600;
pub(crate) fn raster_to_zjs(args: &[String]) -> Result<(), String> {
    // The table path is a value, not a positional argument, so it is left out of the list.
    let table_path = args
        .iter()
        .position(|a| a == "--tone-table")
        .and_then(|i| args.get(i + 1));
    let positional: Vec<&String> = args
        .iter()
        .filter(|a| !a.starts_with('-') && Some(*a) != table_path)
        .collect();
    let (inp, outp) = match positional.as_slice() {
        [i, o] => (*i, *o),
        _ => {
            return Err(
                "raster-to-zjs needs <in> <out> [--no-pjl] [--2bit --tone-table FILE]".into(),
            )
        }
    };
    let raster = read_file(inp)?;
    // A typo must not quietly produce a default job, which is what ignoring unknown options
    // does: the filter has the same guard for the same reason.
    for a in args {
        if a.starts_with('-') && !matches!(a.as_str(), "--no-pjl" | "--2bit" | "--tone-table") {
            return Err(format!("unknown option {a:?}"));
        }
    }
    // --2bit screens the page through HP's own tone table instead of thresholding it, which
    // makes the job two bits per pixel. The table comes from the installed driver:
    // scripts/hp-tone-table.py --out DIR.
    let tone_table = match args.iter().position(|a| a == "--tone-table") {
        Some(i) => Some(read_file(
            args.get(i + 1).ok_or("--tone-table needs a path")?,
        )?),
        None => None,
    };
    if args.iter().any(|a| a == "--2bit") && tone_table.is_none() {
        return Err("--2bit needs --tone-table FILE (scripts/hp-tone-table.py --out DIR)".into());
    }
    let opts = JobOpts {
        pjl: !args.iter().any(|a| a == "--no-pjl"),
        // One table for both slots: this command is for trying a page out, and which of the two
        // HP's marker selects is the filter's business, not its.
        tone_tables: tone_table.map(|t| p1100_job::ToneTables {
            at_400: t.clone(),
            at_600: t,
        }),
        ..JobOpts::default()
    };
    let job = encode_raster_job(&raster, &opts).map_err(|e| e.to_string())?;
    fs::write(outp, &job).map_err(|e| format!("cannot write {outp}: {e}"))?;
    eprintln!(
        "wrote {} bytes to {outp} ({} raster bytes, pjl={})",
        job.len(),
        raster.len(),
        opts.pjl
    );
    Ok(())
}
pub(crate) fn test_page(args: &[String]) -> Result<(), String> {
    let mut pattern: Option<TestPattern> = None;
    let mut page = "letter";
    let mut dpi = DEFAULT_TEST_DPI;
    let mut out = "test-page.zjs".to_string();
    let mut raw = false;
    let mut pjl = true;
    let mut list = false;

    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> Result<&String, String> {
            args.get(i + 1)
                .ok_or_else(|| format!("{} needs a value", args[i]))
        };
        match args[i].as_str() {
            "--page" => {
                page = need(i)?.as_str();
                i += 2;
            }
            "--dpi" => {
                dpi = need(i)?.parse().map_err(|_| "bad --dpi")?;
                i += 2;
            }
            "--raw" => {
                raw = true;
                i += 1;
            }
            "--no-pjl" => {
                pjl = false;
                i += 1;
            }
            "--list" => {
                list = true;
                i += 1;
            }
            "-o" | "--out" => {
                out = need(i)?.clone();
                i += 2;
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other:?}")),
            name => {
                pattern = Some(
                    TestPattern::parse(name)
                        .ok_or_else(|| format!("unknown pattern {name:?}; try --list"))?,
                );
                i += 1;
            }
        }
    }

    if list || pattern.is_none() {
        let names: Vec<&str> = TestPattern::ALL.iter().map(|p| p.name()).collect();
        println!("patterns: {}", names.join(" "));
        let pages: Vec<&str> = TEST_PAGES.iter().map(|(n, _, _, _)| *n).collect();
        println!("pages:    {}", pages.join(" "));
        return if list {
            Ok(())
        } else {
            Err("test-page needs a pattern".into())
        };
    }
    let pattern = pattern.unwrap();

    let (_, w0, h0, paper) = TEST_PAGES
        .iter()
        .find(|(n, _, _, _)| *n == page)
        .ok_or_else(|| format!("unknown page {page:?}"))?;
    let (mut width, mut height) = (*w0, *h0);
    if dpi != DEFAULT_TEST_DPI {
        // scale the measured 600 dpi geometry rather than recomputing from the PPD
        let f = f64::from(dpi) / f64::from(DEFAULT_TEST_DPI);
        width = (f64::from(width) * f).round() as u32;
        height = (f64::from(height) * f).round() as u32;
    }

    p1100_job::bitmap_header(width, height, dpi).map_err(|e| e.to_string())?;
    let bitmap = p1100_job::test_page_bitmap(pattern, width, height);
    let opts = JobOpts {
        encoding: if raw {
            p1100_job::ImageEncoding::Raw
        } else {
            p1100_job::ImageEncoding::Jbig
        },
        pjl,
        ..JobOpts::default()
    };
    let job = p1100_job::encode_bitmap_job(&bitmap, width, height, dpi, *paper, &opts)
        .map_err(|e| e.to_string())?;
    fs::write(&out, &job).map_err(|e| format!("cannot write {out}: {e}"))?;
    eprintln!(
        "{} page: {} at {} dpi -> {}x{} px, {} bytes of 1bpp; job {} bytes -> {}",
        pattern.name(),
        page,
        dpi,
        width,
        height,
        bitmap.len(),
        job.len(),
        out
    );
    Ok(())
}
// The one test here writes files, which WASI does not support: the CLI itself runs on wasm
// (scripts/wasi-run.mjs does exactly that), but this test is native-only.
#[cfg(all(test, not(target_os = "wasi")))]
mod tests {
    use super::*;
    use p1100_protocol::{decode, item, strip_pjl_framing};
    use p1100_raster::{encode_page_header, PageHeader, SYNC_RASTER3};

    #[test]
    fn generated_pages_reject_unbounded_or_zero_resolution_before_allocation() {
        for dpi in ["0", "4294967295"] {
            let args = ["blank", "--dpi", dpi].map(str::to_owned);
            assert!(test_page(&args).is_err());
        }
    }

    fn tiny_raster(width: u32, height: u32, path: &str) {
        let header = PageHeader {
            cups_width: width,
            cups_height: height,
            cups_bytes_per_line: width,
            ..PageHeader::default()
        };
        let mut r = SYNC_RASTER3.to_ne_bytes().to_vec();
        r.extend_from_slice(&encode_page_header(&header));
        r.extend((0..width * height).map(|i| (i * 8) as u8));
        fs::write(path, &r).expect("write the raster");
    }

    /// A table of the documented shape: 256 grey levels of a 12x12 cell, four levels apart.
    fn tone_table() -> Vec<u8> {
        let mut t = vec![0u8; p1100_raster::TONE_TABLE_LEN];
        for grey in 0..256usize {
            let level = (grey / 85) as u8;
            for row in 0..12usize {
                for col in 0..12usize {
                    t[grey * 48 + row * 4 + col / 4] |= level << (6 - 2 * (col % 4));
                }
            }
        }
        t
    }

    fn item_of(job: &[u8], tag: u16) -> u32 {
        let chunks = decode(strip_pjl_framing(job)).expect("decode the job");
        let it = chunks[1]
            .items
            .iter()
            .find(|i| i.item == tag)
            .expect("item present");
        u32::from_be_bytes([it.payload[0], it.payload[1], it.payload[2], it.payload[3]])
    }

    /// The command line this repository documents, including the two-bit one and the ways it
    /// is supposed to refuse: no table, and a typo.
    #[test]
    fn raster_to_zjs_takes_its_arguments() {
        // A relative scratch directory, not the system temporary one: std::env::temp_dir is not
        // implemented under WASI, and this suite runs on wasm as well as natively.
        let dir = std::path::PathBuf::from("target/p1100-cli-test");
        fs::create_dir_all(&dir).expect("create the scratch directory");
        let raster = dir.join("page.raster");
        let out = dir.join("page.zjs");
        tiny_raster(16, 2, raster.to_str().unwrap());
        let args = |extra: &[&str]| -> Vec<String> {
            let mut v = vec![
                raster.to_string_lossy().to_string(),
                out.to_string_lossy().to_string(),
            ];
            v.extend(extra.iter().map(|s| s.to_string()));
            v
        };

        raster_to_zjs(&args(&["--no-pjl"])).expect("the plain invocation");
        let job = fs::read(&out).unwrap();
        assert_eq!(
            &job[..4],
            p1100_protocol::STREAM_SIGNATURE,
            "no PJL means the body starts at the signature"
        );
        assert_eq!(item_of(&job, item::VIDEO_BPP), 1);

        let table = dir.join("tone.bin");
        fs::write(&table, tone_table()).unwrap();
        raster_to_zjs(&args(&["--2bit", "--tone-table", table.to_str().unwrap()]))
            .expect("the two-bit invocation");
        let job = fs::read(&out).unwrap();
        assert_eq!(item_of(&job, item::VIDEO_BPP), 2);
        assert_eq!(item_of(&job, item::NBIE), 2);
        assert_eq!(
            item_of(&job, item::VIDEO_X),
            8,
            "half of the sixteen-pixel width"
        );

        let err = raster_to_zjs(&args(&["--2bit"])).unwrap_err();
        assert!(err.contains("--tone-table"), "{err}");
        let err = raster_to_zjs(&args(&["--wat"])).unwrap_err();
        assert!(err.contains("--wat"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
