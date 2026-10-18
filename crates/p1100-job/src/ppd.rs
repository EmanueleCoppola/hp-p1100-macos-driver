//! Reading print options: CUPS's own option string first, then the PPD's defaults.

use p1100_protocol::{duplex as zduplex, source};

use crate::options::Options;

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
/// | `InputSlot` | `Manual`, `Upper`, ... | `ZJI_DMDEFAULTSOURCE` |
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
        apply_one_option(opts, key, value);
    }
}

/// Apply one `key=value` pair, from CUPS's options string or from a PPD default.
fn apply_one_option(opts: &mut Options, key: &str, value: &str) {
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
        "p1100Grey" => opts.screened = value == "Screened2",
        _ => {}
    }
}

/// Apply the PPD's own defaults for the options this filter maps, before the command line.
///
/// CUPS passes only the options the job actually named: filter(7) says so outright - "options
/// passed on the command-line typically do not include the default choices [of] the printer's
/// PPD file" - and points a filter at `PPD` in the environment and at `ppdMarkDefaults` plus
/// `ppdFindMarkedChoice` to fill the gaps. HP's driver does exactly that; its imports are how
/// we know. Without it, a job that names no options - the common case, since CUPS sends only
/// what the user changed - would send our defaults rather than the printer's configured ones.
///
/// The installed HP PPD declares its only choice as `Manual`, but the P1102's normal paper
/// tray is the one-source path (1). Treating that PPD marker as a default manual-feed request
/// puts the physical printer into attention state for an ordinary tray job. Explicit source
/// choices still override this built-in default below.
pub fn apply_ppd_defaults(opts: &mut Options, ppd: &str) {
    for line in ppd.lines() {
        let Some(rest) = line.strip_prefix("*Default") else {
            continue;
        };
        let Some((key, value)) = rest.split_once(':') else {
            continue;
        };
        // Only the options this filter maps; the rest of the PPD - page size, media type,
        // resolution - reaches us through the raster page header, which already reflects them.
        if matches!(
            key,
            "Duplex" | "EconoMode" | "pjlDensity" | "pjlJamRecovery" | "p1100Grey"
        ) {
            apply_one_option(opts, key, value.trim().trim_matches('"'));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::DEFAULT_DENSITY;
    use crate::test_support::with_options;
    use p1100_protocol::{duplex as zduplex, source};

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
    fn ppd_defaults_fill_in_what_the_command_line_leaves_out() {
        let ppd = "\
*DefaultColorSpace: Gray
*DefaultPageSize: Letter
*DefaultDuplex: None
*DefaultInputSlot: Manual
*DefaultpjlDensity: 3
*DefaultpjlJamRecovery: False
*DefaultEconoMode: False
*DefaultFont: Courier
";
        let mut o = Options::default();
        apply_ppd_defaults(&mut o, ppd);
        // The PPD's Manual marker is intentionally ignored as a default: on the P1102 it
        // requests manual feed and raises the attention light for an ordinary tray job.
        assert_eq!(o.source, source::ONLY_ONE);
        assert_eq!(o.duplex, zduplex::OFF);
        assert_eq!(o.density, 3);
        assert!(!o.econo_mode);
        assert!(!o.jam_recovery_auto);

        // The command line is applied after the file, so it wins.
        apply_ppd_options(&mut o, "InputSlot=Upper pjlDensity=5");
        assert_eq!(o.source, source::ONLY_ONE);
        assert_eq!(o.density, 5);
    }

    /// Only the options we map are read: page size, media type and resolution reach us in the
    /// raster page header, which already reflects the PPD's defaults for them.
    #[test]
    fn ppd_defaults_ignore_the_keywords_we_do_not_map() {
        let mut o = Options::default();
        apply_ppd_defaults(
            &mut o,
            "*DefaultPageSize: A4\n*DefaultResolutions: 600x600dpi\n",
        );
        let d = Options::default();
        assert_eq!(o.source, d.source);
        assert_eq!(o.duplex, d.duplex);
        assert_eq!(o.density, d.density);
        assert_eq!(o.econo_mode, d.econo_mode);
        assert_eq!(o.jam_recovery_auto, d.jam_recovery_auto);
    }

    /// The option that asks for the two-bit screening: named on the command line, and named as
    /// the PPD's default, which is how a job that says nothing at all gets it.
    #[test]
    fn the_grey_option_selects_the_screening() {
        let mut opts = Options::default();
        assert!(!opts.screened, "one bit is the default");

        apply_ppd_options(&mut opts, "p1100Grey=Screened2");
        assert!(opts.screened);

        apply_ppd_options(&mut opts, "p1100Grey=Mono1");
        assert!(!opts.screened);

        let mut opts = Options::default();
        apply_ppd_defaults(&mut opts, "*Defaultp1100Grey: Screened2\n");
        assert!(opts.screened);
    }
}
