//! The job options this filter builds: the queue's PPD, CUPS's option string, then the
//! environment overrides that drive the unverified paths by hand.

use std::env;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use p1100_job::{format_timestamp, ImageEncoding, Options};

/// Build job options from the CUPS options string plus environment overrides.
///
/// The environment overrides exist so the filter can be exercised by hand and so the
/// unverified parts of the encoder can be switched on hardware without a rebuild.
pub(crate) fn build_options(options: &str) -> Options {
    let mut opts = Options {
        timestamp: format_timestamp(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ),
        ..Options::default()
    };

    // CUPS passes only the options this job named, so the PPD supplies the rest - it sets PPD
    // to the queue's file for exactly this. Read it first, so the command line overrides it,
    // which is the order the filter interface documents. A PPD we cannot read is not fatal:
    // the built-in defaults stand, which is what happened before this existed.
    if let Ok(path) = env::var("PPD") {
        match fs::read_to_string(&path) {
            Ok(text) => p1100_job::apply_ppd_defaults(&mut opts, &text),
            Err(e) => {
                eprintln!("WARNING: cannot read PPD {path}: {e}; using built-in defaults")
            }
        }
    }
    p1100_job::apply_ppd_options(&mut opts, options);

    for token in options.split_whitespace() {
        // The PPD's own options are mapped by the library, which is where the mapping can
        // be tested against the real option strings.
        if let Some(v) = token.strip_prefix("P1100Pjl=") {
            opts.pjl = matches!(v, "true" | "True" | "1" | "on");
        }
    }

    if let Ok(v) = env::var("P1100_DENSITY") {
        if let Ok(v) = v.parse::<u32>() {
            opts.density = v;
        }
    }
    if let Ok(v) = env::var("P1100_ECONOMODE") {
        opts.econo_mode = v == "1" || v.eq_ignore_ascii_case("true");
    }
    if let Ok(v) = env::var("P1100_NO_PJL") {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            opts.pjl = false;
        }
    }
    // JBIG compression is the default, as the printer expects. P1100_RAW forces the
    // uncompressed image instead, which exists only to test on hardware whether the
    // printer accepts ZJT_RAW_IMAGE at all.
    if let Ok(v) = env::var("P1100_RAW") {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            opts.encoding = ImageEncoding::Raw;
        }
    }
    opts
}

#[cfg(test)]
mod tests {
    use super::*;
    use p1100_job::DEFAULT_DENSITY;
    use p1100_protocol::duplex as zduplex;

    #[test]
    fn jbig_is_the_default_encoding() {
        assert_eq!(build_options("").encoding, ImageEncoding::Jbig);
    }
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
        let o = build_options("pjlDensity=5 EconoMode=True pjlJamRecovery=True P1100Pjl=false");
        assert_eq!(o.density, 5);
        assert!(o.econo_mode);
        assert!(o.jam_recovery_auto);
        assert!(!o.pjl);
    }
}
