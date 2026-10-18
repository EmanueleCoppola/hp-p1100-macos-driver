//! `p1100` command line tool.
//!
//! Deliberately dependency-free and usable before CUPS integration exists, so the
//! protocol work can be tested on its own:
//!
//! ```text
//! p1100 raster-info   <cups-raster-file>
//! p1100 raster-to-mono1 <cups-raster-file> <out.raw>
//! p1100 raster-to-zjs <cups-raster-file> <out.zjs> [--no-pjl]
//!               [--2bit --tone-table FILE]  screen grey into two bits
//! p1100 zjs-dump      <zjs-file>
//! p1100 wrap --bih <file> [--bid <file> ...] --width W --height H --resolution R \
//!            [--bits N] [--paper-code N] [--no-pjl] -o <out.zjs>
//! ```

mod job;
mod raster;
mod send;
mod zjs;

use std::env;
use std::fs;
use std::process::ExitCode;

use crate::job::{raster_to_zjs, test_page};
use crate::raster::{raster_info, raster_to_mono1};
use crate::send::send;
use crate::zjs::{wrap, zjs_dump};

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
p1100 - HP LaserJet Professional P1100 family tooling

USAGE:
  p1100 raster-info    <cups-raster-file>
  p1100 raster-to-mono1 <cups-raster-file> <out.raw>
  p1100 raster-to-zjs  <cups-raster-file> <out.zjs> [--no-pjl]
                       [--2bit --tone-table FILE]
  p1100 test-page      <pattern> [-o <out.zjs>] [--page letter|a4|legal] [--dpi N]
                                  [--raw] [--no-pjl] [--list]
  p1100 zjs-dump       <zjs-file>
  p1100 wrap --bih <file> [--bid <file> ...] --width W --height H --resolution R
             [--bits N] [--paper-code N] [--no-pjl] -o <out.zjs>
  p1100 send <job.zjs> [--device URI] [--queue NAME] [--timeout SECS] [--dry-run]

NOTES:
  raster-to-zjs and test-page produce complete, printable jobs: the page image is JBIG
  compressed by our own encoder. --raw switches to the uncompressed ZJT_RAW_IMAGE form,
  whose layout still needs confirming on hardware.

  test-page generates the deterministic bring-up pages, so the printer can be exercised
  without CUPS or a source document. To send a job with no build and no filter at all,
  point a raw queue at the device and push the file straight through it:

      p1100 test-page square -o /tmp/square.zjs
      sudo lpadmin -p P1100_raw -E -v usb://... -m raw
      lp -d P1100_raw -o raw /tmp/square.zjs

  `wrap` frames an *already encoded* JBIG image, which is how a reference-compressed
  payload can be produced for comparison; see scripts/make-test-job.py. --bits says how many
  bits per pixel the framed image holds, which is what ZJI_VIDEO_BPP, ZJI_NBIE and ZJI_VIDEO_X
  are written from: 1 (the default) for a bi-level page, 2 for the two-bit page HP's driver
  screens grey into (docs/differences.md item 11).";

/// Read a whole file, with the error message every command reports.
pub(crate) fn read_file(path: &str) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))
}
