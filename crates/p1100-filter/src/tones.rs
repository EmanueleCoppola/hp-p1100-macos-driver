//! Finding the two tone tables the two-bit screening needs.

use std::env;
use std::fs;
use std::path::Path;

fn read_tables(dir: &Path) -> Result<p1100_job::ToneTables, String> {
    let read = |name: &str| -> Result<Vec<u8>, String> {
        let path = dir.join(name);
        let bytes = fs::read(&path).map_err(|e| {
            format!(
                "p1100Grey=Screened2 needs {}, and reading it failed: {e}",
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

/// Load HP's two tone tables when they are installed, and otherwise build the compatible
/// clustered-dot tables into the filter. The built-in path is deliberate: it means Screened2
/// works on a machine that never had HP's driver, and no HP data has to be redistributed.
///
/// `P1100_TONES` is an explicit override, so a bad value there is an error rather than a
/// reason to print with different screening than the caller asked for.
pub(crate) fn load_tone_tables() -> Result<p1100_job::ToneTables, String> {
    if let Ok(dir) = env::var("P1100_TONES") {
        return read_tables(Path::new(&dir)).map_err(|e| {
            format!("{e}. Run 'scripts/hp-tone-table.py --out DIR' and point P1100_TONES at it")
        });
    }

    let dir = env::current_exe()
        .map_err(|e| format!("cannot find this filter's own path: {e}"))?
        .parent()
        .ok_or("this filter is not in a directory")?
        .to_path_buf();

    match read_tables(&dir) {
        Ok(tables) => Ok(tables),
        Err(e) => {
            eprintln!(
                "INFO: no HP tone tables beside {} ({e}); using the built-in clustered-dot \
                 tables",
                dir.display()
            );
            let (at_400, at_600) = p1100_raster::builtin_tone_tables();
            Ok(p1100_job::ToneTables { at_400, at_600 })
        }
    }
}
