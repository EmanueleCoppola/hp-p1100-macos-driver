//! Exercise the CUPS process boundary, with no queue or printer.
#![cfg(not(target_os = "wasi"))]

use std::io::Write;
use std::process::{Command, Stdio};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "macos")]
use std::os::unix::process::CommandExt;
#[cfg(target_os = "macos")]
use std::os::unix::process::ExitStatusExt;

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn dup2(oldfd: i32, newfd: i32) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
}

fn filter() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_p1100-filter"));
    for key in [
        "PPD",
        "P1100_TONES",
        "P1100_DENSITY",
        "P1100_ECONOMODE",
        "P1100_NO_PJL",
        "P1100_RAW",
        "P1100_COMPLETION_TIMEOUT_MS",
    ] {
        cmd.env_remove(key);
    }
    cmd.args(["1", "test", "CUPS boundary", "1", ""]);
    cmd
}

fn raster() -> Vec<u8> {
    let header = p1100_raster::PageHeader {
        cups_width: 8,
        cups_height: 2,
        cups_bytes_per_line: 8,
        num_copies: 3,
        ..Default::default()
    };
    let mut data = p1100_raster::SYNC_RASTER3.to_ne_bytes().to_vec();
    data.extend_from_slice(&p1100_raster::encode_page_header(&header));
    data.extend_from_slice(&[255; 16]);
    data
}

#[cfg(target_os = "macos")]
fn large_raw_raster() -> Vec<u8> {
    let header = p1100_raster::PageHeader {
        cups_width: 4096,
        cups_height: 1024,
        cups_bytes_per_line: 4096,
        ..Default::default()
    };
    let mut data = p1100_raster::SYNC_RASTER3.to_ne_bytes().to_vec();
    data.extend_from_slice(&p1100_raster::encode_page_header(&header));
    data.resize(data.len() + 4096 * 1024, 255);
    data
}

#[test]
fn stdin_and_filename_produce_the_same_job_and_keep_logs_off_stdout() {
    let data = raster();
    let mut child = filter()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&data).unwrap();
    let piped = child.wait_with_output().unwrap();
    assert!(piped.status.success());
    assert!(String::from_utf8_lossy(&piped.stderr).contains("INFO: encoded 1 page(s)"));
    let path = std::env::temp_dir().join(format!("p1100-filter-{}.raster", std::process::id()));
    std::fs::write(&path, data).unwrap();
    let filed = filter().arg(&path).output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(filed.status.success());
    let body = p1100_protocol::strip_pjl_framing(&piped.stdout);
    assert_eq!(body, p1100_protocol::strip_pjl_framing(&filed.stdout));
    let chunks = p1100_protocol::decode(body).unwrap();
    let copies = chunks[1]
        .items
        .iter()
        .find(|i| i.item == p1100_protocol::item::DMCOPIES)
        .unwrap();
    assert_eq!(
        copies.payload,
        3u32.to_be_bytes(),
        "raster NumCopies owns the copy count"
    );
}

#[test]
fn invalid_input_fails_without_printer_output() {
    let mut child = filter()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"not raster")
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).starts_with("ERROR:"));
}

#[cfg(target_os = "macos")]
#[test]
fn sigterm_stops_a_blocked_job_without_a_side_channel_reset() {
    let path = std::env::temp_dir().join(format!("p1100-filter-side-{}", std::process::id()));
    let side = std::fs::File::create(&path).unwrap();
    let side_fd = side.as_raw_fd();
    let mut cmd = filter();
    cmd.env("P1100_RAW", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(move || {
            if dup2(side_fd, 4) == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let raster_path = path.with_extension("raster");
    std::fs::write(&raster_path, large_raw_raster()).unwrap();
    cmd.arg(&raster_path);
    let child = cmd.spawn().unwrap();
    // Debug binaries can spend a moment under macOS's first-run policy check. The raw job is
    // larger than the pipe, so once started the child remains blocked until this signal arrives.
    std::thread::sleep(Duration::from_secs(1));
    unsafe {
        assert_eq!(kill(child.id() as i32, 15), 0);
    }
    let output = child.wait_with_output().unwrap();
    drop(side);
    let side_data = std::fs::read(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(raster_path).unwrap();

    // HP's filter has no SIGTERM handler: CUPS kills it and the backend closes the USB
    // endpoint. Reproducing that is what avoids a soft reset racing the endpoint close while
    // the printer is already stopped for paper.
    assert_eq!(output.status.signal(), Some(15));
    assert!(
        side_data.is_empty(),
        "the filter wrote to the CUPS side channel while being canceled"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_job_stays_open_until_the_printer_reports_the_end_of_the_job() {
    use std::os::unix::net::UnixStream;

    // CUPS hands a filter fd 3 as the backchannel the backend copies printer status to. A
    // socket pair stands in for it here, so the test can play the printer's part. This is the
    // behaviour that keeps the row in Print Center while the sheets come out: the job is not
    // finished while this filter is still running.
    let (mut printer, backchannel) = UnixStream::pair().unwrap();
    let backchannel_fd = backchannel.as_raw_fd();
    let mut cmd = filter();
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(move || {
            if dup2(backchannel_fd, 3) == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = cmd.spawn().unwrap();
    // Dropping the handle closes stdin, which is the filter's end of the raster stream.
    child.stdin.take().unwrap().write_all(&raster()).unwrap();

    std::thread::sleep(Duration::from_millis(750));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the filter finished the job before the printer had printed it"
    );

    printer.write_all(b"@PJL USTATUS JOB\r\nSTART\r\n").unwrap();
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the start of a job is not the end of one"
    );

    printer.write_all(b"@PJL USTATUS JOB\r\nEND\r\n").unwrap();
    let output = child.wait_with_output().unwrap();
    drop(printer);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stderr.contains("INFO: encoded 1 page(s)"), "{stderr}");
    assert!(!stderr.contains("WARNING:"), "{stderr}");
    // The status session is closed the way HP's endJob does it, after the printer has said
    // the job is out. Leaving it open keeps the printer volunteering status into the next
    // job, including one that was canceled while waiting for paper.
    assert!(
        output.stdout.ends_with(&p1100_protocol::pjl_status_off()),
        "the job does not close the printer status session"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_canceled_job_ends_the_wait_immediately() {
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    // The printer says CANCELED when it has given up on a job, which is what a paper-out
    // condition does once the job is canceled at the host. Treating that as progress used to
    // hold the queue for the full silence timeout, and the next job waited behind it.
    let (mut printer, backchannel) = UnixStream::pair().unwrap();
    let backchannel_fd = backchannel.as_raw_fd();
    let mut cmd = filter();
    cmd.env("P1100_COMPLETION_TIMEOUT_MS", "5000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(move || {
            if dup2(backchannel_fd, 3) == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(&raster()).unwrap();
    printer
        .write_all(b"@PJL USTATUS JOB\r\nSTART\r\n@PJL USTATUS JOB\r\nCANCELED\r\n")
        .unwrap();

    let started = Instant::now();
    let output = child.wait_with_output().unwrap();
    let elapsed = started.elapsed();
    drop(printer);

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(elapsed < Duration::from_secs(3), "waited {elapsed:?}");
    assert!(stderr.contains("printer canceled the job"), "{stderr}");
    assert!(
        output.stdout.ends_with(&p1100_protocol::pjl_status_off()),
        "the canceled job did not close the printer status session"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_stale_canceled_status_does_not_end_the_next_job() {
    use std::os::unix::net::UnixStream;

    // The backend can still forward the previous job's final status when the next job
    // starts. A CANCELED with no START for this job is that leftover, not this job's result.
    let (mut printer, backchannel) = UnixStream::pair().unwrap();
    let backchannel_fd = backchannel.as_raw_fd();
    let mut cmd = filter();
    cmd.env("P1100_COMPLETION_TIMEOUT_MS", "5000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(move || {
            if dup2(backchannel_fd, 3) == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(&raster()).unwrap();
    printer
        .write_all(b"@PJL USTATUS JOB\r\nCANCELED\r\n")
        .unwrap();
    std::thread::sleep(Duration::from_millis(750));
    assert!(
        child.try_wait().unwrap().is_none(),
        "a stale CANCELED ended the next job"
    );

    printer
        .write_all(b"@PJL USTATUS JOB\r\nSTART\r\n@PJL USTATUS JOB\r\nEND\r\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    drop(printer);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("printer canceled the job"), "{stderr}");
    assert!(output.stdout.ends_with(&p1100_protocol::pjl_status_off()));
}

#[cfg(target_os = "macos")]
#[test]
fn a_printer_that_only_reports_device_status_does_not_hold_the_queue() {
    use std::os::unix::net::UnixStream;

    // The prologue asks for timed device reports as well as job ones. A printer that answers
    // those but never says the job is finished must not be able to keep the queue busy, so the
    // wait is bounded by silence about the job. The shortened timeout is what makes that
    // reachable in a test rather than after a minute and a half.
    let (mut printer, backchannel) = UnixStream::pair().unwrap();
    let backchannel_fd = backchannel.as_raw_fd();
    let mut cmd = filter();
    cmd.env("P1100_COMPLETION_TIMEOUT_MS", "300")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        cmd.pre_exec(move || {
            if dup2(backchannel_fd, 3) == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(&raster()).unwrap();
    printer
        .write_all(b"@PJL USTATUS DEVICE\r\nCODE=10001\r\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();
    drop(printer);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stderr.contains("WARNING:"), "{stderr}");
    assert!(stderr.contains("INFO: encoded 1 page(s)"), "{stderr}");
    assert!(
        output.stdout.ends_with(&p1100_protocol::pjl_status_off()),
        "the job did not close the printer status session after the timeout"
    );
}
