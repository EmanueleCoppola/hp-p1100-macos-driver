//! What the caller wants printed: the job options and the two screening tone tables.
//!
//! Options arrive from three places, in increasing order of priority: this crate's defaults,
//! the printer's PPD, and the CUPS command line (see ~~crate::ppd~~).

use p1100_protocol::{duplex as zduplex, source};
use p1100_raster::{PageHeader, DEFAULT_THRESHOLD};

/// PJL density default, matching the printer's normal setting.
pub const DEFAULT_DENSITY: u32 = 3;

/// How a page image is presented to the printer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEncoding {
    /// JBIG (T.82) compressed image, the format the printer expects. This is the default
    /// and what a real job uses.
    Jbig,
    /// `ZJT_RAW_IMAGE`: the uncompressed 1 bpp image, same bitmap the compressed path
    /// would feed to JBIG. The payload layout is a hypothesis that still has to be
    /// confirmed on hardware - see `docs/protocol.md`.
    Raw,
}

/// The two tone tables HP's driver chooses between, by the marker its PPD puts in the raster
/// header: the 400 dpi cell and the 600 dpi one.
#[derive(Debug, Clone)]
pub struct ToneTables {
    pub at_400: Vec<u8>,
    pub at_600: Vec<u8>,
}

impl ToneTables {
    /// Which table a page uses. `cupsCompression` is the marker HP's PPD writes through the
    /// page device - 401 for the 600x400dpi choice, 1 for 600x600dpi - and HP's driver reads
    /// it back as `bits = (compression & 1) + 1` and `vdpi = compression & 0x3ffe`. The mask
    /// is what tells 400 dpi from 600, and the 400 dpi engine is the one that wants the
    /// coarser cell.
    pub fn for_page(&self, header: &PageHeader) -> &[u8] {
        if header.cups_compression & 0x3ffe == 400 {
            &self.at_400
        } else {
            &self.at_600
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Wrap the stream in the PJL prologue/epilogue.
    pub pjl: bool,
    pub density: u32,
    pub econo_mode: bool,
    /// `pjlJamRecovery`: true selects `AUTO`, false `OFF`.
    pub jam_recovery_auto: bool,
    /// The `p1100Grey` option this repository's PPD offers: `Screened2` asks for the two-bit
    /// screening HP's driver does, `Mono1` (the default) for the one-bit thresholding the
    /// reference driver sends. The tables themselves are the caller's to supply, because only
    /// the filter knows where they are installed.
    pub screened: bool,
    /// Input tray, from the PPD's `InputSlot`.
    pub source: u32,
    /// Timestamp for the `JOBATTR` line, `YYYYMMDDHHMMSS`.
    pub timestamp: String,
    pub duplex: u32,
    pub encoding: ImageEncoding,
    pub threshold: u8,
    /// The tone tables to screen grey through, HP's own, which make the page two bits per
    /// pixel instead of one. `scripts/hp-tone-table.py --out DIR` writes these files out of
    /// the installed driver; `None` is the one-bit thresholding path.
    pub tone_tables: Option<ToneTables>,
    /// Override `DMPAPER`; `None` derives it from the raster's page size name.
    pub paper: Option<u32>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            pjl: true,
            density: DEFAULT_DENSITY,
            econo_mode: false,
            jam_recovery_auto: false,
            screened: false,
            source: source::ONLY_ONE,
            timestamp: "19700101000000".to_string(),
            duplex: zduplex::OFF,
            encoding: ImageEncoding::Jbig,
            threshold: DEFAULT_THRESHOLD,
            tone_tables: None,
            paper: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{compression_header, table};

    /// The marker HP's PPD leaves in the raster header picks the table: 401 is the 400 dpi
    /// engine, whose cell is the coarser one, and anything else is the 600 dpi cell.
    #[test]
    fn the_compression_marker_picks_the_table() {
        let tables = ToneTables {
            at_400: table(0),
            at_600: table(3),
        };
        assert_eq!(
            tables.for_page(&compression_header(16, 2, 401)),
            tables.at_400.as_slice()
        );
        assert_eq!(
            tables.for_page(&compression_header(16, 2, 1)),
            tables.at_600.as_slice()
        );
        assert_eq!(
            tables.for_page(&compression_header(16, 2, 0)),
            tables.at_600.as_slice()
        );
    }
}
