//! CUPS filter: `application/vnd.cups-raster` in, ZjStream out.
//!
//! This replaces HP's `rastertozjs` for the HP LaserJet Professional P1100 family, of which
//! the P1102 is the model it was developed and verified against. It is a native ARM64
//! binary, where HP's is x86_64-only and cannot launch on this Mac at all.
//!
//! CUPS invokes filters as:
//!
//! ```text
//! <filter> job-id user title copies options [file]
//! ```
//!
//! The raster arrives on stdin, or in `file` when CUPS supplies one, and the encoded job
//! is written to stdout. Diagnostics go to stderr, which CUPS records in the job log.

mod completion;
mod options;
mod side_channel;
mod status;
mod tones;
mod writer;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::atomic::Ordering;

use p1100_job::encode_raster_stream;

use crate::completion::wait_for_printing_to_finish;
use crate::options::build_options;
use crate::side_channel::{install_signal_handlers, request_backend_reset, CANCELED};
use crate::tones::load_tone_tables;
use crate::writer::Counting;

/// The calling convention, for the case where somebody runs this by hand rather than through
/// CUPS.
const USAGE: &str = "\
usage: p1100-filter job-id user title copies options [file]

This is a CUPS filter: the raster arrives on stdin, or in `file` when CUPS supplies one, and
the encoded ZjStream job is written to stdout. `options` is CUPS's space-separated option
string, which carries the PPD settings (Duplex=..., InputSlot=Manual, pjlDensity=...,
EconoMode=..., pjlJamRecovery=...).

To drive the pipeline outside CUPS, use the offline tools instead:
  p1100 --help                inspect rasters, dump streams, build a test page
  scripts/preflight-usb.py    what the USB bus and CUPS see
  scripts/rehearse-job.py     the whole CUPS -> filter boundary, with no printer";
fn main() -> ExitCode {
    install_signal_handlers();
    let args: Vec<String> = env::args().collect();
    // CUPS passes five job arguments and an optional filename. A shorter one is running by
    // hand, and without this guard the filter writes half a job to stdout before failing, which
    // is a memorable way to learn the calling convention.
    let asked_for_usage = args.get(1).is_some_and(|a| a == "-h" || a == "--help");
    if asked_for_usage || !(6..=7).contains(&args.len()) {
        // Requested help is not an error, and CUPS keeps stderr in the job log, so the two
        // cases write to different streams and exit differently.
        if asked_for_usage {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    // argv[5] is the CUPS options string, argv[6] an optional input file path.
    let options = args.get(5).cloned().unwrap_or_default();
    let input_file = args.get(6).filter(|p| !p.is_empty()).cloned();

    let mut opts = build_options(&options);
    // The two-bit screening needs HP's tone tables, which are files rather than options: they
    // live beside this binary, where install.sh puts them, or wherever P1100_TONES points.
    if opts.screened {
        match load_tone_tables() {
            Ok(tables) => opts.tone_tables = Some(tables),
            Err(e) => return fail(&e),
        }
    }
    let stdout = io::stdout();
    let mut out = Counting::new(io::BufWriter::new(stdout.lock()));

    // Streamed: the raster is read a page at a time and each page is written out as soon as
    // it is encoded, so a long document costs a page of memory rather than the whole job.
    let result = match input_file.as_deref() {
        Some(path) => match fs::File::open(path) {
            Ok(file) => encode_raster_stream(file, &mut out, &opts),
            Err(e) => return fail(&format!("opening {path}: {e}")),
        },
        None => encode_raster_stream(io::stdin().lock(), &mut out, &opts),
    };

    if CANCELED.load(Ordering::Relaxed) {
        request_backend_reset();
        return fail("job canceled by CUPS");
    }

    match result {
        Ok(pages) => {
            if CANCELED.load(Ordering::Relaxed) {
                request_backend_reset();
                return fail("job canceled by CUPS");
            }
            if let Err(e) = out.flush() {
                return fail(&format!("writing the job to stdout: {e}"));
            }
            // Hold the job open until the printer reports that the sheets are out. CUPS keeps
            // a job running while one of its filters still is, which is what leaves the row
            // in Print Center until the page has actually printed.
            if opts.pjl {
                if let Err(e) = wait_for_printing_to_finish() {
                    return fail(&e);
                }
            }
            eprintln!(
                // INFO rather than DEBUG: this becomes the queue's last state message, so a
                // successful job leaves a trace of what the filter did with it
                "INFO: encoded {pages} page(s), {} bytes (pjl={}, encoding={:?})",
                out.bytes(),
                opts.pjl,
                opts.encoding
            );
            ExitCode::SUCCESS
        }
        Err(e) => fail(&format!("encoding job: {e}")),
    }
}

fn fail(message: &str) -> ExitCode {
    // CUPS classifies a filter's stderr by its first word: "ERROR: ..." sets
    // printer-state-message and logs at error level, where a line prefixed with the program
    // name is merely stored as text. So the prefix goes first, as Apple's own filters do.
    eprintln!("ERROR: {message}");
    ExitCode::from(1)
}
