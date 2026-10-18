//! Holding the job open until the printer reports that the sheets are out.

#[cfg(not(target_os = "wasi"))]
use std::env;
#[cfg(not(target_os = "wasi"))]
use std::sync::atomic::Ordering;
#[cfg(not(target_os = "wasi"))]
use std::time::{Duration, Instant};

#[cfg(not(target_os = "wasi"))]
use crate::side_channel::{
    backchannel_is_open, poll_backchannel, read_backchannel, request_backend_reset, CANCELED,
    POLLHUP, POLLIN, POLLNVAL,
};
#[cfg(not(target_os = "wasi"))]
use crate::status::{job_status_progress, pjl_job_finished};

/// How long the filter waits for the next sign that the job is moving. The printer reports
/// each page as the sheet comes out, so this bounds silence about the job rather than the
/// length of the job: a long document keeps resetting it, a printer that has stopped talking
/// about the job does not.
#[cfg(not(target_os = "wasi"))]
const COMPLETION_SILENCE_TIMEOUT: Duration = Duration::from_secs(90);

/// How long the filter waits for the *first* sign that the printer has taken the job.
///
/// A printer that is working reports within a second or two of the transfer. Waiting the whole
/// silence window for a printer that says nothing at all is what leaves the queue sitting
/// behind a job that is not going to finish; the log of 18 October showed exactly that, with
/// one job stopped as unresponsive while the next waited its 90 seconds out.
#[cfg(not(target_os = "wasi"))]
const COMPLETION_FIRST_SIGNAL_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest a job can be held open in total, however chatty the printer is. The prologue
/// also asks for `USTATUS TIMED`, which makes the printer volunteer device status every 30
/// seconds; without a total limit, a printer that reports those but never ends the job would
/// keep the queue busy forever.
#[cfg(not(target_os = "wasi"))]
const COMPLETION_HARD_LIMIT: Duration = Duration::from_secs(300);

/// How long the wait tolerates silence about the job. `P1100_COMPLETION_TIMEOUT_MS` shortens
/// it, which is how the process test reaches the give-up path without waiting the full time.
#[cfg(not(target_os = "wasi"))]
fn completion_silence_timeout() -> Duration {
    env::var("P1100_COMPLETION_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(COMPLETION_SILENCE_TIMEOUT)
}
/// Wait on the CUPS backchannel until the printer reports that the job has been printed.
///
/// This is what HP's filter does at the end of a job: it writes `@PJL EOJ`, joins its
/// backchannel reader, and only then sends `@PJL USTATUSOFF`. Because CUPS keeps a job open
/// while one of its filters is still running, that wait is what holds the row in Print Center
/// until the sheets are out instead of letting it disappear when the bytes reach the printer.
///
/// The prologue asks for `@PJL USTATUS JOB = ON`, so the printer reports `START` and, once
/// the pages are printed, `END`. Returns whether that `END` was seen. The wait is bounded in
/// two ways, because a filter that waits can hold a queue open: by silence about the job, and
/// by a total limit on the job itself.
#[cfg(not(target_os = "wasi"))]
fn wait_for_printer_completion() -> Result<bool, String> {
    let started = Instant::now();
    let silence = completion_silence_timeout();
    let mut status = Vec::new();
    let mut previous_tail: Vec<u8> = Vec::new();
    // The first sign gets the shorter limit: silence before the printer has said anything
    // about the job means it is not going to, and the queue must not wait it out.
    let mut deadline = started + silence.min(COMPLETION_FIRST_SIGNAL_TIMEOUT);
    loop {
        if CANCELED.load(Ordering::Relaxed) {
            request_backend_reset();
            return Err("job canceled by CUPS".into());
        }
        if Instant::now() >= deadline || started.elapsed() >= COMPLETION_HARD_LIMIT {
            return Ok(false);
        }
        let (ready, revents) = poll_backchannel(1000);
        if ready < 0 {
            continue; // SIGTERM/SIGPIPE set CANCELED and the next iteration reports it.
        }
        if ready > 0 && revents & POLLIN != 0 {
            let mut bytes = [0u8; 4096];
            let count = read_backchannel(&mut bytes);
            if count <= 0 {
                // The backend closed the backchannel, so no further status can arrive.
                return Ok(false);
            }
            let chunk = &bytes[..count as usize];
            // The printer's messages are short and the backend can split one across reads, so
            // the search for progress runs over the end of the last read plus this one.
            let mut window = previous_tail.clone();
            window.extend_from_slice(chunk);
            if job_status_progress(&window) {
                deadline = Instant::now() + silence;
            }
            previous_tail.clear();
            previous_tail.extend_from_slice(&chunk[chunk.len().saturating_sub(16)..]);

            status.extend_from_slice(chunk);
            if pjl_job_finished(&status) {
                return Ok(true);
            }
            // The job-status messages are short. Retain just enough tail to join a message
            // the backend split, without letting a chatty printer grow this without bound.
            if status.len() > 8192 {
                status.drain(..status.len() - 512);
            }
            continue;
        }
        if ready > 0 && revents & POLLHUP != 0 {
            return Ok(false);
        }
        if ready > 0 && revents & POLLNVAL != 0 {
            return Ok(false); // no such descriptor is open, so there is nothing to wait on
        }
    }
}
/// Hold the job open until the printer has printed it, then close out the PJL session.
///
/// The filter keeps stdout open while it waits, exactly as HP's does: the backend is still
/// forwarding the printer's status bytes to this process on fd 3, and a filter that has
/// already exited is a job CUPS considers finished.
#[cfg(not(target_os = "wasi"))]
pub(crate) fn wait_for_printing_to_finish() -> Result<(), String> {
    if !backchannel_is_open() {
        return Ok(()); // run by hand, or rehearsed: there is no backchannel to read
    }
    if !wait_for_printer_completion()? {
        eprintln!(
            "WARNING: the printer did not report the end of the job; the job ends without \
             that notification"
        );
    }
    // Nothing is written after the wait. HP's filter sends `@PJL USTATUSOFF` here, but that
    // opens a second PJL session after the job the printer has just finished; leaving the
    // status reports on costs a few bytes at the timed interval and cannot leave the printer
    // waiting for the header of a job that will never come.
    Ok(())
}

#[cfg(target_os = "wasi")]
pub(crate) fn wait_for_printing_to_finish() -> Result<(), String> {
    Ok(())
}
