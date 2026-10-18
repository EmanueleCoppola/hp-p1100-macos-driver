//! Finding and reading the two tone tables the two-bit screening needs.

use std::env;
use std::fs;
use std::path::PathBuf;

/// The two tone tables the two-bit screening needs: from `P1100_TONES` when it is set, and
/// from beside this binary otherwise, which is where `scripts/install.sh` puts them.
pub(crate) fn load_tone_tables() -> Result<p1100_job::ToneTables, String> {
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
