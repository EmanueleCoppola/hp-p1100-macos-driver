//! Formatting a clock reading for the PJL ~~JOBATTR~~ line.

/// Format a Unix timestamp as `YYYYMMDDHHMMSS` in UTC, for the PJL `JOBATTR` line.
///
/// Done by hand rather than with a date crate: the value is cosmetic, and this keeps the
/// dependency list empty. Returns the epoch string for values that overflow the calendar.
pub fn format_timestamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = unix_secs % 86_400;
    // Civil-from-days, Hinnant's algorithm, with the epoch shifted to 0000-03-01.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    if !(0..=9999).contains(&y) {
        return "19700101000000".to_string();
    }
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        y,
        m,
        d,
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::format_timestamp;

    #[test]
    fn formats_known_instants() {
        assert_eq!(format_timestamp(0), "19700101000000");
        assert_eq!(format_timestamp(1), "19700101000001");
        assert_eq!(format_timestamp(86_400), "19700102000000");
        // 2000-03-01T00:00:00Z, just after a leap day.
        assert_eq!(format_timestamp(951_868_800), "20000301000000");
        // 2026-09-15T12:34:56Z
        assert_eq!(format_timestamp(1_789_475_696), "20260915123456");
    }
}
