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

use std::env;
#[cfg(not(target_os = "wasi"))]
use std::ffi::c_void;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(target_os = "wasi"))]
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use p1100_job::{encode_raster_stream, format_timestamp, ImageEncoding, Options};

static CANCELED: AtomicBool = AtomicBool::new(false);

#[cfg(not(target_os = "wasi"))]
const CUPS_SC_FD: i32 = 4;
#[cfg(not(target_os = "wasi"))]
const CUPS_BC_FD: i32 = 3;
#[cfg(not(target_os = "wasi"))]
const CUPS_SC_CMD_SOFT_RESET: u8 = 1;
#[cfg(not(target_os = "wasi"))]
const POLLIN: i16 = 0x0001;
#[cfg(not(target_os = "wasi"))]
const POLLHUP: i16 = 0x0010;
#[cfg(not(target_os = "wasi"))]
const POLLNVAL: i16 = 0x0020;
#[cfg(not(target_os = "wasi"))]
const F_GETFD: i32 = 1;

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

#[cfg(not(target_os = "wasi"))]
unsafe extern "C" {
    fn signal(signum: i32, handler: Option<extern "C" fn(i32)>) -> Option<extern "C" fn(i32)>;
    fn siginterrupt(signum: i32, flag: i32) -> i32;
    // `link_name` rather than a bare name: declaring a Rust item called `read` or `write`
    // defines a symbol the standard library also looks for, and the compiler warns that the
    // definition it would pick up is suspect. The C functions are the ones meant here.
    #[link_name = "write"]
    fn c_write(fd: i32, buffer: *const c_void, count: usize) -> isize;
    #[link_name = "read"]
    fn c_read(fd: i32, buffer: *mut c_void, count: usize) -> isize;
    fn poll(fds: *mut PollFd, nfds: u32, timeout: i32) -> i32;
    fn fcntl(fd: i32, cmd: i32) -> i32;
    fn _exit(status: i32) -> !;
}

#[cfg(not(target_os = "wasi"))]
#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[cfg(not(target_os = "wasi"))]
extern "C" fn canceled(_: i32) {
    CANCELED.store(true, Ordering::Relaxed);
    // CUPS side-channel messages are command, status, big-endian length, data. Writing the
    // request here is async-signal-safe; the normal path below repeats it after encoding stops.
    let request = [CUPS_SC_CMD_SOFT_RESET, 0, 0, 0];
    unsafe {
        let _ = c_write(CUPS_SC_FD, request.as_ptr().cast(), request.len());
        // A filter can be blocked in a restarted stdout write, so returning from the handler
        // is not enough to let the cancellation reach CUPS. The reset request is complete and
        // _exit is async-signal-safe.
        _exit(1);
    }
}

#[cfg(not(target_os = "wasi"))]
fn install_signal_handlers() {
    unsafe {
        signal(15, Some(canceled)); // SIGTERM: CUPS cancellation/hold
        signal(13, Some(canceled)); // SIGPIPE: downstream backend closed the job
                                    // Do not transparently restart a blocked stdout write: the next write check must see
                                    // CANCELED and let the filter leave the pipeline promptly.
        siginterrupt(15, 1);
        siginterrupt(13, 1);
    }
}

#[cfg(target_os = "wasi")]
fn install_signal_handlers() {}

#[cfg(not(target_os = "wasi"))]
fn request_backend_reset() {
    let request = [CUPS_SC_CMD_SOFT_RESET, 0, 0, 0];
    unsafe {
        let _ = c_write(CUPS_SC_FD, request.as_ptr().cast(), request.len());
    }
}

#[cfg(target_os = "wasi")]
fn request_backend_reset() {}

/// Return whether a PJL status byte stream contains the end of the current job.
/// PJL itself permits either LF or CRLF, while the USB backend can split a message anywhere.
fn pjl_job_finished(data: &[u8]) -> bool {
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
fn job_status_progress(data: &[u8]) -> bool {
    [b"USTATUS JOB".as_slice(), b"USTATUS PAGE".as_slice()]
        .iter()
        .any(|marker| data.windows(marker.len()).any(|window| window == *marker))
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
        let mut pollfd = PollFd {
            fd: CUPS_BC_FD,
            events: POLLIN,
            revents: 0,
        };
        let ready = unsafe { poll(&mut pollfd, 1, 1000) };
        if ready < 0 {
            continue; // SIGTERM/SIGPIPE set CANCELED and the next iteration reports it.
        }
        if ready > 0 && pollfd.revents & POLLIN != 0 {
            let mut bytes = [0u8; 4096];
            let count = unsafe { c_read(CUPS_BC_FD, bytes.as_mut_ptr().cast(), bytes.len()) };
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
        if ready > 0 && pollfd.revents & POLLHUP != 0 {
            return Ok(false);
        }
        if ready > 0 && pollfd.revents & POLLNVAL != 0 {
            return Ok(false); // no such descriptor is open, so there is nothing to wait on
        }
    }
}

/// Whether CUPS handed this filter a backchannel to read the printer's status from.
///
/// cupsd opens fd 3 for a job's filters; the announcement tools that run the filter by hand
/// do not, and neither does the offline rehearsal. Asking rather than assuming is what keeps
/// those runs from waiting for a notification that cannot arrive.
#[cfg(not(target_os = "wasi"))]
fn backchannel_is_open() -> bool {
    (unsafe { fcntl(CUPS_BC_FD, F_GETFD) }) >= 0
}

/// Hold the job open until the printer has printed it, then close out the PJL session.
///
/// The filter keeps stdout open while it waits, exactly as HP's does: the backend is still
/// forwarding the printer's status bytes to this process on fd 3, and a filter that has
/// already exited is a job CUPS considers finished.
#[cfg(not(target_os = "wasi"))]
fn wait_for_printing_to_finish() -> Result<(), String> {
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
fn wait_for_printing_to_finish() -> Result<(), String> {
    Ok(())
}

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
                out.bytes, opts.pjl, opts.encoding
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

/// Forwards writes and counts them, so the filter can report the size of the job it produced
/// even though it never holds the whole thing.
struct Counting<W: Write> {
    inner: W,
    bytes: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if CANCELED.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "job canceled by CUPS",
            ));
        }
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
    use p1100_job::DEFAULT_DENSITY;
    use p1100_protocol::duplex as zduplex;

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

/// The two tone tables the two-bit screening needs: from `P1100_TONES` when it is set, and
/// from beside this binary otherwise, which is where `scripts/install.sh` puts them.
fn load_tone_tables() -> Result<p1100_job::ToneTables, String> {
    let dir = match env::var("P1100_TONES") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => env::current_exe()
            .map_err(|e| format!("cannot find this filter's own path: {e}"))?
            .parent()
            .ok_or("this filter is not in a directory")?
            .to_path_buf(),
    };
    let read = |name: &str| -> Result<Vec<u8>, String> {
        let path = dir.join(name);
        let bytes = fs::read(&path).map_err(|e| {
            format!(
                "p1100Grey=Screened2 needs {}, and reading it failed: {e}. Run \
                 'scripts/hp-tone-table.py --out DIR' and install what it writes, or point \
                 P1100_TONES at it",
                path.display()
            )
        })?;
        if bytes.len() < p1100_raster::TONE_TABLE_LEN {
            return Err(format!(
                "{} is {} bytes; a whole tone table is {}",
                path.display(),
                bytes.len(),
                p1100_raster::TONE_TABLE_LEN
            ));
        }
        Ok(bytes)
    };
    Ok(p1100_job::ToneTables {
        at_400: read("p1100-400.tones")?,
        at_600: read("p1100-600.tones")?,
    })
}
