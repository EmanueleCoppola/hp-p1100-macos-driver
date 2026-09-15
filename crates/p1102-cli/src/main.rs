//! `p1102` command line tool.
//!
//! Deliberately dependency-free and usable before CUPS integration exists, so the
//! protocol work can be tested on its own:
//!
//! ```text
//! p1102 raster-info   <cups-raster-file>
//! p1102 raster-to-mono1 <cups-raster-file> <out.raw>
//! p1102 raster-to-zjs <cups-raster-file> <out.zjs> [--no-pjl]
//!               [--2bit --tone-table FILE]  screen grey into two bits
//! p1102 zjs-dump      <zjs-file>
//! p1102 wrap --bih <file> [--bid <file> ...] --width W --height H --resolution R \
//!            [--bits N] [--paper-code N] [--no-pjl] -o <out.zjs>
//! ```

use std::env;
use std::fs;
use std::process::{Command, ExitCode};

use p1102_job::{encode_raster_job, Options as JobOpts, TestPattern};
use p1102_protocol::{
    decode, op_name, pjl_epilogue, pjl_prologue, strip_pjl_framing, Encoder, JobOptions,
    PageGeometry, PjlSettings,
};
use p1102_raster::{to_mono1, RasterReader, DEFAULT_THRESHOLD};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let rest = &args[1..];
    let result = match cmd {
        "raster-info" => raster_info(rest),
        "raster-to-mono1" => raster_to_mono1(rest),
        "raster-to-zjs" => raster_to_zjs(rest),
        "test-page" => test_page(rest),
        "zjs-dump" => zjs_dump(rest),
        "wrap" => wrap(rest),
        "send" => send(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
p1102 - HP LaserJet Professional P1102 tooling

USAGE:
  p1102 raster-info    <cups-raster-file>
  p1102 raster-to-mono1 <cups-raster-file> <out.raw>
  p1102 raster-to-zjs  <cups-raster-file> <out.zjs> [--no-pjl]
                       [--2bit --tone-table FILE]
  p1102 test-page      <pattern> [-o <out.zjs>] [--page letter|a4|legal] [--dpi N]
                                  [--raw] [--no-pjl] [--list]
  p1102 zjs-dump       <zjs-file>
  p1102 wrap --bih <file> [--bid <file> ...] --width W --height H --resolution R
             [--bits N] [--paper-code N] [--no-pjl] -o <out.zjs>
  p1102 send <job.zjs> [--device URI] [--queue NAME] [--timeout SECS] [--dry-run]

NOTES:
  raster-to-zjs and test-page produce complete, printable jobs: the page image is JBIG
  compressed by our own encoder. --raw switches to the uncompressed ZJT_RAW_IMAGE form,
  whose layout still needs confirming on hardware.

  test-page generates the deterministic bring-up pages, so the printer can be exercised
  without CUPS or a source document. To send a job with no build and no filter at all,
  point a raw queue at the device and push the file straight through it:

      p1102 test-page square -o /tmp/square.zjs
      sudo lpadmin -p P1102_raw -E -v usb://... -m raw
      lp -d P1102_raw -o raw /tmp/square.zjs

  `wrap` frames an *already encoded* JBIG image, which is how a reference-compressed
  payload can be produced for comparison; see scripts/make-test-job.py. --bits says how many
  bits per pixel the framed image holds, which is what ZJI_VIDEO_BPP, ZJI_NBIE and ZJI_VIDEO_X
  are written from: 1 (the default) for a bi-level page, 2 for the two-bit page HP's driver
  screens grey into (docs/differences.md item 11).";

/// Apple's own CUPS backend: the transport a queue uses, world-executable, and the only thing
/// that has to run to put a job on the wire. `p1102 send` wraps it so the bring-up notes are one
/// command rather than a DEVICE_URI dance.
const USB_BACKEND: &str = "/usr/libexec/cups/backend/usb";

/// Measured 600 dpi page geometry, the same page images cgpdftoraster produces.
const TEST_PAGES: [(&str, u32, u32, u32); 3] = [
    ("letter", 4900, 6400, 1),
    ("a4", 4758, 6817, 9),
    ("legal", 4900, 8100, 5),
];
const DEFAULT_TEST_DPI: u32 = 600;

// ---------------------------------------------------------------------------

fn read_file(path: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))
}

fn raster_info(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("raster-info needs a file")?;
    let data = read_file(path)?;
    let mut reader = RasterReader::new(&data);
    let mut n = 0;
    while let Some((h, px)) = reader.next_page().map_err(|e| format!("{e:?}"))? {
        n += 1;
        println!(
            "page {n}: {}x{} px, {} bpc/{} bpp, {} bytes/line, {} colour(s), page {:?}",
            h.cups_width,
            h.cups_height,
            h.cups_bits_per_color,
            h.cups_bits_per_pixel,
            h.cups_bytes_per_line,
            h.cups_num_colors,
            h.cups_page_size_name
        );
        println!(
            "        media type {:?}, resolution {:?} dpi, margins {:?} pt, copies {}",
            h.media_type, h.hw_resolution, h.margins, h.num_copies
        );
        println!(
            "        mono row stride {} bytes (128-bit aligned), JBIG width {} (128 px aligned), payload {} bytes",
            h.mono_row_bytes(),
            h.jbig_width(),
            px.len()
        );
        println!(
            "        page size {:?} pt, bbox {:?}, borderless {}, cupsCompression {}",
            h.cups_page_size, h.cups_imaging_bbox, h.cups_borderless_scaling, h.cups_compression
        );
    }
    println!("pages: {n}");
    Ok(())
}

fn raster_to_mono1(args: &[String]) -> Result<(), String> {
    let (inp, outp) = match args {
        [i, o] => (i, o),
        _ => return Err("raster-to-mono1 needs <in> <out>".into()),
    };
    let data = read_file(inp)?;
    let mut reader = RasterReader::new(&data);
    let mut all = Vec::new();
    let mut pages = 0;
    while let Some((h, px)) = reader.next_page().map_err(|e| format!("{e:?}"))? {
        pages += 1;
        let mono = to_mono1(&h, px, DEFAULT_THRESHOLD).map_err(|e| format!("{e:?}"))?;
        eprintln!(
            "page {pages}: {}x{} -> {} bytes of 1bpp (stride {})",
            h.cups_width,
            h.cups_height,
            mono.len(),
            h.mono_row_bytes()
        );
        all.extend_from_slice(&mono);
    }
    fs::write(outp, &all).map_err(|e| format!("cannot write {outp}: {e}"))?;
    eprintln!("wrote {} bytes to {outp} ({pages} page(s))", all.len());
    Ok(())
}

fn zjs_dump(args: &[String]) -> Result<(), String> {
    let path = args.first().ok_or("zjs-dump needs a file")?;
    let raw = read_file(path)?;
    let framed = raw.windows(4).any(|w| w == b"@PJL");
    let data = strip_pjl_framing(&raw);
    if framed {
        println!("PJL framing present; decoding the ZjStream body only");
    }
    let chunks = decode(data).map_err(|e| format!("{e}"))?;
    let mut total_items = 0usize;
    let mut payload = 0usize;
    for (i, c) in chunks.iter().enumerate() {
        println!(
            "[{i:03}] {:<11} size={:<7} items={:<3} reserved={:#06x} sig={:#06x}",
            op_name(c.header.ty),
            c.header.size,
            c.header.items,
            c.header.reserved,
            c.header.signature
        );
        for it in &c.items {
            let value = if it.ty == p1102_protocol::item_type::UINT32 && it.payload.len() == 4 {
                format!(
                    "u32={}",
                    u32::from_be_bytes([
                        it.payload[0],
                        it.payload[1],
                        it.payload[2],
                        it.payload[3]
                    ])
                )
            } else if it.ty == p1102_protocol::item_type::STRING {
                let end = it
                    .payload
                    .iter()
                    .position(|b| *b == 0)
                    .unwrap_or(it.payload.len());
                format!("str={:?}", String::from_utf8_lossy(&it.payload[..end]))
            } else {
                format!("{} bytes", it.payload.len())
            };
            println!(
                "       item tag=0x{:04x} type={} size={} param={} {}",
                it.item, it.ty, it.size, it.param, value
            );
            total_items += 1;
        }
        if !c.payload.is_empty() {
            println!("       payload {} bytes", c.payload.len());
        }
        payload += c.payload.len();
    }
    println!(
        "chunks: {}  items: {}  payload bytes: {}",
        chunks.len(),
        total_items,
        payload
    );
    Ok(())
}

fn raster_to_zjs(args: &[String]) -> Result<(), String> {
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
        tone_table,
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

fn test_page(args: &[String]) -> Result<(), String> {
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

    let bitmap = p1102_job::test_page_bitmap(pattern, width, height);
    let opts = JobOpts {
        encoding: if raw {
            p1102_job::ImageEncoding::Raw
        } else {
            p1102_job::ImageEncoding::Jbig
        },
        pjl,
        ..JobOpts::default()
    };
    let job = p1102_job::encode_bitmap_job(&bitmap, width, height, dpi, *paper, &opts)
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

fn wrap(args: &[String]) -> Result<(), String> {
    let mut bih: Option<String> = None;
    let mut bids: Vec<String> = Vec::new();
    let mut out: Option<String> = None;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut resolution = 600u32;
    let mut paper = p1102_protocol::paper::LETTER;
    let mut bits = 1u32;
    let mut pjl = true;

    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> Result<&String, String> {
            args.get(i + 1)
                .ok_or_else(|| format!("{} needs a value", args[i]))
        };
        match args[i].as_str() {
            "--bih" => {
                bih = Some(need(i)?.clone());
                i += 2;
            }
            "--bid" => {
                bids.push(need(i)?.clone());
                i += 2;
            }
            "--width" => {
                width = need(i)?.parse().map_err(|_| "bad --width")?;
                i += 2;
            }
            "--height" => {
                height = need(i)?.parse().map_err(|_| "bad --height")?;
                i += 2;
            }
            "--resolution" => {
                resolution = need(i)?.parse().map_err(|_| "bad --resolution")?;
                i += 2;
            }
            "--paper-code" => {
                paper = need(i)?.parse().map_err(|_| "bad --paper-code")?;
                i += 2;
            }
            "--bits" => {
                bits = need(i)?.parse().map_err(|_| "bad --bits")?;
                if !(1..=4).contains(&bits) {
                    return Err("--bits must be between 1 and 4".into());
                }
                i += 2;
            }
            "--no-pjl" => {
                pjl = false;
                i += 1;
            }
            "-o" | "--out" => {
                out = Some(need(i)?.clone());
                i += 2;
            }
            other => return Err(format!("unknown option {other:?}")),
        }
    }

    let bih_path = bih.ok_or("--bih is required")?;
    let out = out.ok_or("-o is required")?;
    if width == 0 || height == 0 {
        return Err("--width and --height are required".into());
    }
    let bih = read_file(&bih_path)?;
    if bih.len() != p1102_protocol::JBIG_BIH_LEN {
        return Err(format!(
            "{} is {} bytes; the JBIG BIH must be exactly {} bytes",
            bih_path,
            bih.len(),
            p1102_protocol::JBIG_BIH_LEN
        ));
    }
    let mut bid_data = Vec::new();
    for p in &bids {
        bid_data.push(read_file(p)?);
    }
    let bid_refs: Vec<&[u8]> = bid_data.iter().map(|v| v.as_slice()).collect();

    let mut g = PageGeometry::mono(width, height, resolution);
    g.paper = paper;
    g.bits_per_pixel = bits;

    let mut enc = Encoder::new();
    let mut file = Vec::new();
    if pjl {
        file.extend_from_slice(&pjl_prologue(&PjlSettings::default()));
    }
    enc.stream_signature();
    enc.doc_start(&JobOptions::default());
    enc.page_start(&g);
    enc.jbig_image(&bih, &bid_refs);
    enc.end_jbig(g.duplex);
    enc.page_end(g.duplex);
    enc.doc_end();
    file.extend_from_slice(enc.as_bytes());
    if pjl {
        file.extend_from_slice(&pjl_epilogue());
    }
    fs::write(&out, &file).map_err(|e| format!("cannot write {out}: {e}"))?;
    eprintln!(
        "wrote {} bytes to {out} (zjs body {}, {} data chunk(s))",
        file.len(),
        enc.as_bytes().len(),
        bid_refs.len()
    );
    Ok(())
}

// The one test here writes files, which WASI does not support: the CLI itself runs on wasm
// (scripts/wasi-run.mjs does exactly that), but this test is native-only.
#[cfg(all(test, not(target_os = "wasi")))]
mod tests {
    use super::*;
    use p1102_protocol::{decode, item, strip_pjl_framing};
    use p1102_raster::{encode_page_header, PageHeader, SYNC_RASTER3};

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
        let mut t = vec![0u8; p1102_raster::TONE_TABLE_LEN];
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
        let dir = std::path::PathBuf::from("target/p1102-cli-test");
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
            p1102_protocol::STREAM_SIGNATURE,
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

/// Send a job file to the printer through Apple's own backend, the transport a queue would use.
/// No queue and no administrator: the backend takes its device URI from the environment and
/// reads the job from the file argument, as backend(7) describes.
fn send(args: &[String]) -> Result<(), String> {
    let mut job: Option<String> = None;
    let mut device: Option<String> = None;
    let mut queue = "HP_LaserJet_Professional_P1102".to_string();
    let mut timeout = 60u64;
    let mut dry = false;
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize| -> Result<&String, String> {
            args.get(i + 1)
                .ok_or_else(|| format!("{} needs a value", args[i]))
        };
        match args[i].as_str() {
            "--device" => {
                device = Some(need(i)?.clone());
                i += 2;
            }
            "--queue" => {
                queue = need(i)?.clone();
                i += 2;
            }
            "--timeout" => {
                timeout = need(i)?.parse().map_err(|_| "bad --timeout")?;
                i += 2;
            }
            "--dry-run" => {
                dry = true;
                i += 1;
            }
            other if other.starts_with('-') => return Err(format!("unknown option {other:?}")),
            name => {
                if job.is_some() {
                    return Err("send takes one job file".into());
                }
                job = Some(name.to_string());
                i += 1;
            }
        }
    }
    let job = job.ok_or("send needs a job file: p1102 send <job.zjs> [--device URI]")?;
    if !dry && !std::path::Path::new(&job).exists() {
        return Err(format!("no such job file: {job}"));
    }
    let device = match device {
        Some(d) => d,
        None => {
            // The queue already knows the device URI, and reading it is what the bring-up notes
            // do by hand: lpstat -v prints "device for <queue>: <uri>".
            let out = Command::new("lpstat")
                .args(["-v", &queue])
                .output()
                .map_err(|e| format!("cannot run lpstat: {e}"))?;
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .find_map(|l| l.split_once(": "))
                .map(|(_, uri)| uri.trim().to_string())
                .ok_or_else(|| format!("queue {queue} has no device URI; pass --device URI"))?
        }
    };
    let user = env::var("USER").unwrap_or_else(|_| "user".to_string());
    let argv = ["1", user.as_str(), "p1102 send", "1", "", job.as_str()];
    if dry {
        println!("DEVICE_URI={device} {USB_BACKEND} {}", argv.join(" "));
        return Ok(());
    }
    eprintln!("sending {job} to {device} ({timeout}s limit; Ctrl-C to stop)");
    let mut child = Command::new(USB_BACKEND)
        .args(argv)
        .env("DEVICE_URI", &device)
        .spawn()
        .map_err(|e| format!("cannot run {USB_BACKEND}: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
    loop {
        match child
            .try_wait()
            .map_err(|e| format!("waiting for the backend: {e}"))?
        {
            Some(status) if status.success() => {
                eprintln!("the backend accepted the job");
                return Ok(());
            }
            Some(status) => return Err(format!("the backend exited with {status}")),
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "still going after {timeout}s, so the printer is not taking the job - \
                         which is what the backend says with no printer attached, as the \
                         bring-up section of docs/testing.md records"
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
    }
}

#[cfg(test)]
mod send_tests {
    use super::*;

    #[test]
    fn send_takes_one_job_and_refuses_the_rest() {
        let args = |v: &[&str]| -> Vec<String> { v.iter().map(|s| s.to_string()).collect() };

        // A dry run needs no printer and no device: the URI comes from --device.
        send(&args(&["job.zjs", "--dry-run", "--device", "usb://x"])).expect("a dry run");

        let err = send(&args(&[])).unwrap_err();
        assert!(err.contains("needs a job file"), "{err}");

        let err = send(&args(&["a.zjs", "b.zjs", "--dry-run", "--device", "u"])).unwrap_err();
        assert!(err.contains("one job file"), "{err}");

        let err = send(&args(&["job.zjs", "--wat"])).unwrap_err();
        assert!(err.contains("--wat"), "{err}");

        // A file that is not there is refused before anything is spawned.
        let err = send(&args(&["/nonexistent/job.zjs", "--device", "usb://x"])).unwrap_err();
        assert!(err.contains("no such job file"), "{err}");
    }
}
