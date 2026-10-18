//! The PJL framing that wraps the chunk stream.
//!
//! HP's printer wants the job wrapped in PJL: a prologue that turns the status reports on and
//! switches the language to ZJS, and an epilogue that ends the job.

use crate::wire::STREAM_SIGNATURE;

/// The PJL Universal Exit Language sequence.
pub const UEL: &str = "\u{1b}%-12345X";

/// PJL prologue emitted before the `JZJZ` stream signature.
///
/// Matches the framing accepted by HP's macOS driver on the P1102. `timestamp` is passed in
/// rather than read from the clock so the output stays deterministic and testable.
/// PJL settings for a job, all of which the printer's PPD can influence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PjlSettings<'a> {
    /// `pjlDensity`, 1..5 on this printer.
    pub density: u32,
    /// `EconoMode`.
    pub econo_mode: bool,
    /// `pjlJamRecovery`: true selects `AUTO`, false `OFF`.
    pub jam_recovery_auto: bool,
    /// `YYYYMMDDHHMMSS` for the `JOBATTR` line.
    pub timestamp: &'a str,
}

impl<'a> Default for PjlSettings<'a> {
    fn default() -> Self {
        Self {
            density: 3,
            econo_mode: false,
            jam_recovery_auto: false,
            timestamp: "19700101000000",
        }
    }
}

pub fn pjl_prologue(settings: &PjlSettings<'_>) -> Vec<u8> {
    let mut s = String::new();
    s.push_str(UEL);
    s.push_str("@PJL JOB\n");
    s.push_str(&format!(
        "@PJL SET JAMRECOVERY={}\n",
        if settings.jam_recovery_auto {
            "AUTO"
        } else {
            "OFF"
        }
    ));
    s.push_str(&format!("@PJL SET DENSITY={}\n", settings.density));
    s.push_str(&format!(
        "@PJL SET ECONOMODE={}\n",
        if settings.econo_mode { "ON" } else { "OFF" }
    ));
    s.push_str("@PJL SET RET=MEDIUM\n");
    s.push_str("@PJL INFO STATUS\n");
    s.push_str("@PJL USTATUS DEVICE = ON\n");
    s.push_str("@PJL USTATUS JOB = ON\n");
    s.push_str("@PJL USTATUS PAGE = ON\n");
    s.push_str("@PJL USTATUS TIMED = 30\n");
    s.push_str(&format!(
        "@PJL SET JOBATTR=\"JobAttr4={}\"",
        settings.timestamp
    ));
    let mut out = s.into_bytes();
    out.push(0);
    // The P1102 needs the explicit language switch. Ending PJL with a bare UEL makes the USB
    // backend report success, but the printer can enter attention state and emit a blank page.
    out.extend_from_slice(b"@PJL ENTER LANGUAGE=ZJS\n");
    out
}

/// PJL epilogue emitted after `ZJT_END_DOC`.
pub fn pjl_epilogue() -> Vec<u8> {
    format!("{UEL}@PJL EOJ\n{UEL}").into_bytes()
}

/// PJL housekeeping sent once the printer has reported the job as printed.
///
/// The prologue turns the job, page and device status reports on so the printer can announce
/// when the sheets are out; this turns them off again. HP's filter writes the same line at the
/// end of a job, and it keeps one job's status traffic out of the next job's backchannel.
pub fn pjl_status_off() -> Vec<u8> {
    format!("{UEL}@PJL USTATUSOFF\n{UEL}").into_bytes()
}

/// Return the ZjStream body of a job, stripping the PJL prologue and epilogue if present.
///
/// Useful for inspection tools and for tests that want to decode a captured or generated
/// job: [`decode`] is strict about the chunk stream, so the framing has to come off first.
pub fn strip_pjl_framing(data: &[u8]) -> &[u8] {
    let start = match data
        .windows(STREAM_SIGNATURE.len())
        .position(|w| w == STREAM_SIGNATURE)
    {
        Some(p) => p,
        None => return data,
    };
    let rest = &data[start..];
    let marker = format!("{UEL}@PJL EOJ");
    match rest
        .windows(marker.len())
        .position(|w| w == marker.as_bytes())
    {
        Some(p) => &rest[..p],
        None => rest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode, op, Encoder};
    #[test]
    fn strip_pjl_framing_removes_prologue_and_epilogue() {
        let mut job = pjl_prologue(&PjlSettings::default());
        job.extend_from_slice(STREAM_SIGNATURE);
        let mut enc = Encoder::new();
        enc.doc_end();
        job.extend_from_slice(enc.as_bytes());
        job.extend_from_slice(&pjl_epilogue());

        let body = strip_pjl_framing(&job);
        assert_eq!(&body[..4], STREAM_SIGNATURE);
        let chunks = decode(body).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].header.ty, op::END_DOC);
    }

    #[test]
    fn strip_pjl_framing_is_a_no_op_without_framing() {
        let bare = b"JZJZnothing else";
        assert_eq!(strip_pjl_framing(bare), &bare[..]);
    }
    #[test]
    fn prologue_selects_zjs_after_the_nul_terminated_job_attributes() {
        let p = pjl_prologue(&PjlSettings {
            density: 3,
            econo_mode: false,
            jam_recovery_auto: false,
            timestamp: "20260915120000",
        });
        assert!(p.starts_with(UEL.as_bytes()));
        assert!(p.ends_with(b"@PJL ENTER LANGUAGE=ZJS\n"));
        let text = String::from_utf8_lossy(&p);
        assert!(text.contains("@PJL SET DENSITY=3\n"));
        assert!(text.contains("@PJL SET ECONOMODE=OFF\n"));
        assert!(text.contains("@PJL SET JOBATTR=\"JobAttr4=20260915120000\""));
        assert!(text.contains("@PJL ENTER LANGUAGE=ZJS\n"));
        // exactly one NUL, right before the language switch
        assert_eq!(p.iter().filter(|b| **b == 0).count(), 1);
        let nul = p.iter().position(|b| *b == 0).unwrap();
        assert_eq!(&p[nul + 1..], b"@PJL ENTER LANGUAGE=ZJS\n");
    }

    #[test]
    fn econo_mode_toggles() {
        assert!(String::from_utf8_lossy(&pjl_prologue(&PjlSettings {
            density: 1,
            econo_mode: true,
            ..PjlSettings::default()
        }))
        .contains("ECONOMODE=ON"));
    }

    #[test]
    fn jam_recovery_selects_off_or_auto() {
        let off = String::from_utf8_lossy(&pjl_prologue(&PjlSettings::default())).into_owned();
        assert!(off.contains("@PJL SET JAMRECOVERY=OFF\n"));
        let auto = String::from_utf8_lossy(&pjl_prologue(&PjlSettings {
            jam_recovery_auto: true,
            ..PjlSettings::default()
        }))
        .into_owned();
        assert!(auto.contains("@PJL SET JAMRECOVERY=AUTO\n"));
        assert!(!auto.contains("JAMRECOVERY=OFF"));
    }

    #[test]
    fn epilogue_is_uel_eoj_uel() {
        assert_eq!(
            String::from_utf8(pjl_epilogue()).unwrap(),
            format!("{UEL}@PJL EOJ\n{UEL}")
        );
    }

    #[test]
    fn status_off_is_uel_ustatusoff_uel() {
        assert_eq!(
            String::from_utf8(pjl_status_off()).unwrap(),
            format!("{UEL}@PJL USTATUSOFF\n{UEL}")
        );
    }
}
