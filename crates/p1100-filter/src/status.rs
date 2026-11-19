//! Reading the printer's PJL status reports.

/// What the printer has last said about the job the filter submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JobStatus {
    Running,
    Finished,
    Canceled,
}

const JOB_MARKER: &[u8] = b"USTATUS JOB";
const PAGE_MARKER: &[u8] = b"USTATUS PAGE";

fn normalized(data: &[u8]) -> Vec<u8> {
    data.iter().copied().filter(|byte| *byte != b'\r').collect()
}

fn find(data: &[u8], needle: &[u8]) -> Option<usize> {
    data.windows(needle.len())
        .position(|window| window == needle)
}

fn skip_separators(mut data: &[u8]) -> &[u8] {
    while let Some((first, rest)) = data.split_first() {
        if matches!(first, b'\n' | b' ' | b'\t' | b'=') {
            data = rest;
        } else {
            break;
        }
    }
    data
}

fn job_states(data: &[u8]) -> impl Iterator<Item = JobStatus> + '_ {
    let mut search_at = 0;
    std::iter::from_fn(move || {
        let offset = find(&data[search_at..], JOB_MARKER)?;
        let position = search_at + offset;
        search_at = position + JOB_MARKER.len();
        let rest = skip_separators(&data[position + JOB_MARKER.len()..]);
        if rest.starts_with(b"END") {
            Some(JobStatus::Finished)
        } else if rest.starts_with(b"CANCELED") {
            Some(JobStatus::Canceled)
        } else {
            Some(JobStatus::Running)
        }
    })
}

/// The last job status in a run of status bytes.
///
/// Looking at the last message matters: the backend may still hold a previous job's `END`
/// when a new job starts, and that must not be read as the new job finishing. PJL permits
/// either LF or CRLF, and the USB backend can split a message anywhere.
pub(crate) fn job_status(data: &[u8]) -> JobStatus {
    job_states(&normalized(data))
        .last()
        .unwrap_or(JobStatus::Running)
}

/// Whether a run of status bytes shows the job itself moving along.
///
/// `START` and page reports are progress. `CANCELED` and `END` are terminal, so treating
/// them as progress would keep a queue open after the printer has already given up. The
/// timed device reports are not evidence about the job either.
pub(crate) fn job_status_progress(data: &[u8]) -> bool {
    let data = normalized(data);
    if find(&data, PAGE_MARKER).is_some() {
        return true;
    }
    job_started(&data)
}

/// Whether the printer has announced the start of *this* job.
///
/// A terminal message without a preceding `START` can be a leftover from the job that was
/// canceled before this one. Treating it as this job's outcome would let a stale `CANCELED`
/// end a job that has not begun to print yet, which is the opposite of the recovery this wait
/// is meant to allow.
pub(crate) fn job_started(data: &[u8]) -> bool {
    let data = normalized(data);
    let mut search_at = 0;
    while let Some(offset) = find(&data[search_at..], JOB_MARKER) {
        let position = search_at + offset;
        search_at = position + JOB_MARKER.len();
        if skip_separators(&data[position + JOB_MARKER.len()..]).starts_with(b"START") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_end_of_a_job_across_crlf() {
        assert_eq!(
            job_status(b"@PJL USTATUS JOB\r\nEND\r\n"),
            JobStatus::Finished
        );
        assert_eq!(
            job_status(b"noise\nUSTATUS JOB\nEND\n"),
            JobStatus::Finished
        );
        assert_eq!(
            job_status(b"@PJL USTATUS JOB\r\nSTART\r\n"),
            JobStatus::Running
        );
    }

    #[test]
    fn canceled_is_its_own_terminal_state() {
        assert_eq!(
            job_status(b"@PJL USTATUS JOB\r\nCANCELED\r\n"),
            JobStatus::Canceled
        );
        assert_eq!(
            job_status(b"@PJL USTATUS JOB\r\nSTART\r\n@PJL USTATUS JOB\r\nCANCELED\r\n"),
            JobStatus::Canceled
        );
    }

    #[test]
    fn the_last_job_message_wins() {
        // A stale end from the previous job must not finish the job that just started.
        assert_eq!(
            job_status(b"@PJL USTATUS JOB\r\nEND\r\n@PJL USTATUS JOB\r\nSTART\r\n"),
            JobStatus::Running
        );
        assert_eq!(
            job_status(b"@PJL USTATUS JOB\r\nSTART\r\n@PJL USTATUS JOB\r\nEND\r\n"),
            JobStatus::Finished
        );
    }

    #[test]
    fn an_incomplete_status_word_is_not_terminal() {
        assert_eq!(job_status(b"@PJL USTATUS JOB\r\nCA"), JobStatus::Running);
    }

    #[test]
    fn only_job_start_and_page_reports_are_progress() {
        assert!(job_status_progress(b"@PJL USTATUS JOB\r\nSTART\r\n"));
        assert!(job_status_progress(b"@PJL USTATUS PAGE\r\n"));
        assert!(!job_status_progress(b"@PJL USTATUS JOB\r\nCANCELED\r\n"));
        assert!(!job_status_progress(b"@PJL USTATUS JOB\r\nEND\r\n"));
        assert!(!job_status_progress(
            b"@PJL USTATUS DEVICE\r\nCODE=10001\r\n"
        ));
        assert!(!job_status_progress(b"@PJL INFO STATUS\r\nCODE=10001\r\n"));
    }

    #[test]
    fn a_start_is_remembered_separately_from_a_terminal_status() {
        assert!(job_started(b"@PJL USTATUS JOB\r\nSTART\r\n"));
        assert!(!job_started(b"@PJL USTATUS JOB\r\nCANCELED\r\n"));
        assert!(!job_started(b"@PJL USTATUS PAGE\r\n"));
    }
}
