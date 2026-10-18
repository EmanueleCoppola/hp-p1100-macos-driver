//! What the caller says about the document as a whole.

use crate::duplex;

/// Job-level options for the `ZJT_START_DOC` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobOptions {
    pub duplex: u32,
    /// Emitted only when known. The reference encoder omits it on the P1102 path, which
    /// CUPS never supplies a total for.
    pub page_count: Option<u32>,
    /// Emitted only when set. The reference encoder omits it on the P1102 path.
    pub quantity: Option<u32>,
    pub collate: bool,
}

impl Default for JobOptions {
    fn default() -> Self {
        Self {
            duplex: duplex::OFF,
            page_count: None,
            quantity: None,
            collate: false,
        }
    }
}
