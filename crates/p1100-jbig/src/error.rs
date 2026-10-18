//! Why an image cannot be encoded.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JbigError {
    /// The bitmap is smaller than width x height requires.
    BitmapTooSmall { need: usize, got: usize },
    /// The number of bitmaps does not match the number of bit planes.
    PlaneCount { expected: u8, got: usize },
    /// Width or height was zero.
    Empty,
    /// The image's byte count is not representable on this target.
    GeometryOverflow,
}

impl std::fmt::Display for JbigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JbigError::BitmapTooSmall { need, got } => {
                write!(f, "bitmap too small: need {need} bytes, got {got}")
            }
            JbigError::PlaneCount { expected, got } => {
                write!(f, "{got} bitmap(s) for a {expected}-plane image")
            }
            JbigError::Empty => write!(f, "image has zero width or height"),
            JbigError::GeometryOverflow => write!(f, "image byte count overflows usize"),
        }
    }
}

impl std::error::Error for JbigError {}
