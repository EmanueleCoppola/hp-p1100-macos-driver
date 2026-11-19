//! The CUPS backchannel: the printer's status stream, which the backend exposes on fd 3.
//!
//! This module deliberately does not handle signals. HP's filter does not either: when CUPS
//! cancels a job it terminates the filter, and the backend closes the USB endpoint. Sending a
//! side-channel soft reset from the filter as it exits is a second USB operation racing that
//! close, and a printer that is already stopped for paper does not need it. Matching the
//! original here is what lets the next job recover.

#[cfg(not(target_os = "wasi"))]
use std::ffi::c_void;

#[cfg(not(target_os = "wasi"))]
const CUPS_BC_FD: i32 = 3;
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
    // `link_name` rather than a bare name: declaring a Rust item called `read` defines a
    // symbol the standard library also looks for, and the compiler warns that the definition
    // it would pick up is suspect. The C function is the one meant here.
    #[link_name = "read"]
    fn c_read(fd: i32, buffer: *mut c_void, count: usize) -> isize;
    fn poll(fds: *mut PollFd, nfds: u32, timeout: i32) -> i32;
    fn fcntl(fd: i32, cmd: i32) -> i32;
}

#[cfg(not(target_os = "wasi"))]
#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

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
