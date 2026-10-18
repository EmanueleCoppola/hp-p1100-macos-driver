//! The CUPS side channel, and the signals that end a job.
//!
//! cupsd opens fd 3 (backchannel, the printer's status) and fd 4 (side channel, our commands)
//! for a job's filters. Both are plain file descriptors, so this module is the one place that
//! talks to them, and the raw C declarations live here rather than being scattered through the
//! filter.

#[cfg(not(target_os = "wasi"))]
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static CANCELED: AtomicBool = AtomicBool::new(false);
#[cfg(not(target_os = "wasi"))]
const CUPS_SC_FD: i32 = 4;
#[cfg(not(target_os = "wasi"))]
const CUPS_BC_FD: i32 = 3;
#[cfg(not(target_os = "wasi"))]
const CUPS_SC_CMD_SOFT_RESET: u8 = 1;
#[cfg(not(target_os = "wasi"))]
pub(crate) const POLLIN: i16 = 0x0001;
#[cfg(not(target_os = "wasi"))]
pub(crate) const POLLHUP: i16 = 0x0010;
#[cfg(not(target_os = "wasi"))]
pub(crate) const POLLNVAL: i16 = 0x0020;
#[cfg(not(target_os = "wasi"))]
const F_GETFD: i32 = 1;
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
pub(crate) fn install_signal_handlers() {
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
pub(crate) fn install_signal_handlers() {}

#[cfg(not(target_os = "wasi"))]
pub(crate) fn request_backend_reset() {
    let request = [CUPS_SC_CMD_SOFT_RESET, 0, 0, 0];
    unsafe {
        let _ = c_write(CUPS_SC_FD, request.as_ptr().cast(), request.len());
    }
}

#[cfg(target_os = "wasi")]
pub(crate) fn request_backend_reset() {}
/// Whether CUPS handed this filter a backchannel to read the printer's status from.
///
/// cupsd opens fd 3 for a job's filters; the announcement tools that run the filter by hand
/// do not, and neither does the offline rehearsal. Asking rather than assuming is what keeps
/// those runs from waiting for a notification that cannot arrive.
#[cfg(not(target_os = "wasi"))]
pub(crate) fn backchannel_is_open() -> bool {
    (unsafe { fcntl(CUPS_BC_FD, F_GETFD) }) >= 0
}

/// Wait up to `timeout_ms` for the backchannel, and report what `poll(2)` said about it: its
/// return value, then the descriptor's readiness flags.
#[cfg(not(target_os = "wasi"))]
pub(crate) fn poll_backchannel(timeout_ms: i32) -> (i32, i16) {
    let mut pollfd = PollFd {
        fd: CUPS_BC_FD,
        events: POLLIN,
        revents: 0,
    };
    let ready = unsafe { poll(&mut pollfd, 1, timeout_ms) };
    (ready, pollfd.revents)
}

/// Read whatever the backend has forwarded from the printer. Zero or less means the
/// backchannel is closed.
#[cfg(not(target_os = "wasi"))]
pub(crate) fn read_backchannel(buf: &mut [u8]) -> isize {
    unsafe { c_read(CUPS_BC_FD, buf.as_mut_ptr().cast(), buf.len()) }
}
