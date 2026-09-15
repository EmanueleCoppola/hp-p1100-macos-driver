//! CUPS filter: `application/vnd.cups-raster` in, ZjStream out.
//!
//! This replaces HP's `rastertozjs` for the HP LaserJet Professional P1102. It is a
//! native ARM64 binary with no `libcups` dependency, so it is not affected by the
//! removed CUPS libraries that stop HP's filter from running at all.
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
use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use p1005_job::{encode_raster_job, format_timestamp, ImageEncoding, Options};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    // argv[5] is the CUPS options string, argv[6] an optional input file path.
    let options = args.get(5).cloned().unwrap_or_default();
    let input_file = args.get(6).filter(|p| !p.is_empty()).cloned();

    let raster = match read_input(input_file.as_deref()) {
        Ok(v) => v,
        Err(e) => return fail(&format!("reading raster input: {e}")),
    };

    let opts = build_options(&options);
    match encode_raster_job(&raster, &opts) {
        Ok(bytes) => {
            let stdout = io::stdout();
            let mut out = stdout.lock();
            if let Err(e) = out.write_all(&bytes).and_then(|()| out.flush()) {
                return fail(&format!("writing job to stdout: {e}"));
            }
            eprintln!(
                "p1005-filter: encoded {} bytes ({} raster bytes, pjl={}, encoding={:?})",
                bytes.len(),
                raster.len(),
                opts.pjl,
                opts.encoding
            );
            ExitCode::SUCCESS
        }
        Err(e) => fail(&format!("encoding job: {e}")),
    }
}

fn fail(message: &str) -> ExitCode {
    eprintln!("p1005-filter: ERROR: {message}");
    ExitCode::from(1)
}

fn read_input(path: Option<&str>) -> io::Result<Vec<u8>> {
    match path {
        Some(p) => fs::read(p),
        None => {
            let mut buf = Vec::new();
            io::stdin().lock().read_to_end(&mut buf)?;
            Ok(buf)
        }
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
    p1005_job::apply_ppd_options(&mut opts, options);

    for token in options.split_whitespace() {
        // The PPD's own options are mapped by the library, which is where the mapping can
        // be tested against the real option strings.
        if let Some(v) = token.strip_prefix("P1005Pjl=") {
            opts.pjl = matches!(v, "true" | "True" | "1" | "on");
        }
    }

    if let Ok(v) = env::var("P1005_DENSITY") {
        if let Ok(v) = v.parse::<u32>() {
            opts.density = v;
        }
    }
    if let Ok(v) = env::var("P1005_ECONOMODE") {
        opts.econo_mode = v == "1" || v.eq_ignore_ascii_case("true");
    }
    if let Ok(v) = env::var("P1005_NO_PJL") {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            opts.pjl = false;
        }
    }
    // JBIG compression is the default, as the printer expects. P1005_RAW forces the
    // uncompressed image instead, which exists only to test on hardware whether the
    // printer accepts ZJT_RAW_IMAGE at all.
    if let Ok(v) = env::var("P1005_RAW") {
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
    use p1005_job::DEFAULT_DENSITY;
    use p1005_protocol::duplex as zduplex;

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
        let o = build_options("pjlDensity=5 EconoMode=True pjlJamRecovery=True P1005Pjl=false");
        assert_eq!(o.density, 5);
        assert!(o.econo_mode);
        assert!(o.jam_recovery_auto);
        assert!(!o.pjl);
    }
}
