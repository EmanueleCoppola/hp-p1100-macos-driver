//! The one error type this crate reports to its callers.

use p1100_raster::RasterError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Raster(RasterError),
    Jbig(p1100_jbig::JbigError),
    /// Writing the job out failed.
    Io(String),
    /// The raster stream contained no pages.
    NoPages,
    /// A combination of options this build cannot produce.
    Unsupported(String),
}

impl From<RasterError> for Error {
    fn from(e: RasterError) -> Self {
        Error::Raster(e)
    }
}

impl From<p1100_jbig::JbigError> for Error {
    fn from(e: p1100_jbig::JbigError) -> Self {
        Error::Jbig(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Raster(e) => write!(f, "{e}"),
            Error::Jbig(e) => write!(f, "compressing the page image: {e}"),
            Error::Io(e) => write!(f, "writing the job: {e}"),
            Error::NoPages => write!(f, "the raster stream contained no pages"),
            Error::Unsupported(what) => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for Error {}
