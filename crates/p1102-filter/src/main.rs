//! CUPS filter: `application/vnd.cups-raster` in, ZjStream out.
//!
//! This replaces HP's `rastertozjs` for the HP LaserJet Professional P1102. It is a native
//! ARM64 binary, where HP's is x86_64-only and cannot launch on this Mac at all.
//!
//! CUPS invokes filters as:
//!
//! ```text
//! <filter> job-id user title copies options [file]
//! ```
//!
//! The raster arrives on stdin, or in `file` when CUPS supplies one, and the encoded job
//! is written to stdout. Diagnostics go to stderr, which CUPS records in the job log.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use p1102_job::{encode_raster_stream, format_timestamp, ImageEncoding, Options};

/// The calling convention, for the case where somebody runs this by hand rather than through
/// CUPS.
const USAGE: &str = "\
usage: p1102-filter job-id user title copies options [file]

This is a CUPS filter: the raster arrives on stdin, or in `file` when CUPS supplies one, and
the encoded ZjStream job is written to stdout. `options` is CUPS's space-separated option
string, which carries the PPD settings (Duplex=..., InputSlot=Manual, pjlDensity=...,
EconoMode=..., pjlJamRecovery=...).

To drive the pipeline outside CUPS, use the offline tools instead:
  p1102 --help                inspect rasters, dump streams, build a test page
  scripts/preflight-usb.py    what the USB bus and CUPS see
  scripts/rehearse-job.py     the whole CUPS -> filter boundary, with no printer";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    // CUPS always passes all seven arguments. A shorter one is somebody running the filter by
    // hand, and without this guard the filter writes half a job to stdout before failing, which
    // is a memorable way to learn the calling convention.
    let asked_for_usage = args.get(1).is_some_and(|a| a == "-h" || a == "--help");
    if asked_for_usage || args.len() < 6 {
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

    let opts = build_options(&options);
    let stdout = io::stdout();
    let mut out = Counting {
        inner: io::BufWriter::new(stdout.lock()),
        bytes: 0,
    };

    // Streamed: the raster is read a page at a time and each page is written out as soon as
    // it is encoded, so a long document costs a page of memory rather than the whole job.
    let result = match input_file.as_deref() {
        Some(path) => match fs::File::open(path) {
            Ok(file) => encode_raster_stream(file, &mut out, &opts),
            Err(e) => return fail(&format!("opening {path}: {e}")),
        },
        None => encode_raster_stream(io::stdin().lock(), &mut out, &opts),
    };

    match result {
        Ok(pages) => {
            if let Err(e) = out.flush() {
                return fail(&format!("writing the job to stdout: {e}"));
            }
            eprintln!(
                "p1102-filter: encoded {pages} page(s), {} bytes (pjl={}, encoding={:?})",
                out.bytes, opts.pjl, opts.encoding
            );
            ExitCode::SUCCESS
        }
        Err(e) => fail(&format!("encoding job: {e}")),
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("p1102-filter: ERROR: {message}");
    ExitCode::from(1)
}

/// Forwards writes and counts them, so the filter can report the size of the job it produced
/// even though it never holds the whole thing.
struct Counting<W: Write> {
    inner: W,
    bytes: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Build job options from the CUPS options string plus environment overrides.
///
/// The environment overrides exist so the filter can be exercised by hand and so the
/// unverified parts of the encoder can be switched on hardware without a rebuild.
fn build_options(options: &str) -> Options {
    let mut opts = Options {
        timestamp: format_timestamp(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ),
        ..Options::default()
    };
    p1102_job::apply_ppd_options(&mut opts, options);

    for token in options.split_whitespace() {
        // The PPD's own options are mapped by the library, which is where the mapping can
        // be tested against the real option strings.
        if let Some(v) = token.strip_prefix("P1102Pjl=") {
            opts.pjl = matches!(v, "true" | "True" | "1" | "on");
        }
    }

    if let Ok(v) = env::var("P1102_DENSITY") {
        if let Ok(v) = v.parse::<u32>() {
            opts.density = v;
        }
    }
    if let Ok(v) = env::var("P1102_ECONOMODE") {
        opts.econo_mode = v == "1" || v.eq_ignore_ascii_case("true");
    }
    if let Ok(v) = env::var("P1102_NO_PJL") {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            opts.pjl = false;
        }
    }
    // JBIG compression is the default, as the printer expects. P1102_RAW forces the
    // uncompressed image instead, which exists only to test on hardware whether the
    // printer accepts ZJT_RAW_IMAGE at all.
    if let Ok(v) = env::var("P1102_RAW") {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            opts.encoding = ImageEncoding::Raw;
        }
    }
    opts
}

#[cfg(test)]
mod encoding_tests {
    use super::*;

    #[test]
    fn jbig_is_the_default_encoding() {
        assert_eq!(build_options("").encoding, ImageEncoding::Jbig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p1102_job::DEFAULT_DENSITY;
    use p1102_protocol::duplex as zduplex;

    #[test]
    fn defaults_are_the_printers_normal_settings() {
        let o = build_options("");
        assert_eq!(o.density, DEFAULT_DENSITY);
        assert!(!o.econo_mode);
        assert!(o.pjl);
        assert_eq!(o.duplex, zduplex::OFF);
    }

    #[test]
    fn sides_option_selects_duplex() {
        assert_eq!(
            build_options("sides=two-sided-long-edge").duplex,
            zduplex::LONG_EDGE
        );
        assert_eq!(
            build_options("media=A4 Duplex=DuplexNoTumble").duplex,
            zduplex::LONG_EDGE
        );
        assert_eq!(build_options("sides=one-sided").duplex, zduplex::OFF);
        assert_eq!(build_options("Duplex=None").duplex, zduplex::OFF);
    }

    #[test]
    fn unknown_options_are_ignored() {
        let o = build_options("media=A4 copies=2 InputSlot=Auto someflag");
        assert_eq!(o.density, DEFAULT_DENSITY);
        assert_eq!(o.duplex, zduplex::OFF);
    }

    #[test]
    fn explicit_options_override_defaults() {
        let o = build_options("pjlDensity=5 EconoMode=True pjlJamRecovery=True P1102Pjl=false");
        assert_eq!(o.density, 5);
        assert!(o.econo_mode);
        assert!(o.jam_recovery_auto);
        assert!(!o.pjl);
    }
}
