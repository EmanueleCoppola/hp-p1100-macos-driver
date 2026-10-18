//! Reading the printer's PJL status reports.

/// Return whether a PJL status byte stream contains the end of the current job.
/// PJL itself permits either LF or CRLF, while the USB backend can split a message anywhere.
pub(crate) fn pjl_job_finished(data: &[u8]) -> bool {
    let normalized: Vec<u8> = data.iter().copied().filter(|byte| *byte != b'\r').collect();
    normalized
        .windows(b"USTATUS JOB\nEND".len())
        .any(|window| window == b"USTATUS JOB\nEND")
}

/// Whether a run of status bytes shows the job itself moving along.
///
/// HP's reader watches the same two families of message. The timed device reports are not
/// evidence of progress, and treating them as such is what would let a printer hold a queue
/// open by talking about something other than the job.
pub(crate) fn job_status_progress(data: &[u8]) -> bool {
    [b"USTATUS JOB".as_slice(), b"USTATUS PAGE".as_slice()]
        .iter()
        .any(|marker| data.windows(marker.len()).any(|window| window == *marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_a_pjl_job_end_across_crlf_status_lines() {
        assert!(pjl_job_finished(b"@PJL USTATUS JOB\r\nEND\r\n"));
        assert!(pjl_job_finished(b"noise\nUSTATUS JOB\nEND\n"));
        assert!(!pjl_job_finished(b"@PJL USTATUS JOB\r\nSTART\r\n"));
    }
    #[test]
    fn only_the_end_of_job_status_line_finishes_the_job() {
        // A bare "END" from any other status message must not be read as the end of job, and
        // the notification may arrive split across two reads of the backchannel.
        assert!(!pjl_job_finished(
            b"@PJL INFO STATUS\r\nCODE=10001\r\nDISPLAY=\"READY\"\r\n"
        ));
        assert!(!pjl_job_finished(b"READY\nEND\n"));
        assert!(!pjl_job_finished(
            b"@PJL USTATUS JOB\r\nSTART\r\nready\r\n@PJL USTATUS"
        ));
        assert!(!pjl_job_finished(
            b"@PJL USTATUS JOB\r\nSTART\r\nready\r\n@PJL USTATUS JOB\r\n"
        ));
        assert!(pjl_job_finished(
            b"@PJL USTATUS JOB\r\nSTART\r\nready\r\n@PJL USTATUS JOB\r\nEND\r\n"
        ));
    }
    #[test]
    fn only_job_status_counts_as_progress() {
        assert!(job_status_progress(b"@PJL USTATUS JOB\r\nSTART\r\n"));
        assert!(job_status_progress(b"@PJL USTATUS PAGE\r\n"));
        // The timed device reports the prologue also asks for say nothing about the job, so
        // they must not be able to keep a queue open on their own.
        assert!(!job_status_progress(
            b"@PJL USTATUS DEVICE\r\nCODE=10001\r\n"
        ));
        assert!(!job_status_progress(b"@PJL INFO STATUS\r\nCODE=10001\r\n"));
    }
}
