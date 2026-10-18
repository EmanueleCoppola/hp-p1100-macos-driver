//! The stdout writer: it counts what it forwards, so the filter can report the size of a job
//! it never holds, and it stops when CUPS cancels the job.

use std::io::{self, Write};
use std::sync::atomic::Ordering;

use crate::side_channel::CANCELED;

/// Forwards writes and counts them, so the filter can report the size of the job it produced
/// even though it never holds the whole thing.
pub(crate) struct Counting<W: Write> {
    inner: W,
    bytes: u64,
}

impl<W: Write> Counting<W> {
    /// Wrap a writer and start counting.
    pub(crate) fn new(inner: W) -> Self {
        Self { inner, bytes: 0 }
    }

    /// How many bytes have been forwarded so far.
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
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
