//! Write the two screening tables the filter carries, so an installer can put them beside
//! it instead of making the filter build them on every job.

use std::fs;
use std::path::Path;

use p1100_raster::builtin_tone_tables;

pub fn tones(args: &[String]) -> Result<(), String> {
    let dir = match args {
        [dir] => Path::new(dir),
        _ => return Err("usage: p1100 tones <dir>".to_string()),
    };
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let (at_400, at_600) = builtin_tone_tables();
    for (name, table) in [("p1100-400.tones", at_400), ("p1100-600.tones", at_600)] {
        let path = dir.join(name);
        fs::write(&path, &table).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!("{}", path.display());
    }
    Ok(())
}
