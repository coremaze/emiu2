//! Timestamped operational logging to standard output.
//!
//! One line per lifecycle event (connections, pairings, disconnects);
//! nothing per relayed message, which would be thousands of lines a
//! second during play.

use std::time::{SystemTime, UNIX_EPOCH};

macro_rules! log {
    ($($arg:tt)*) => {
        $crate::logging::print(format_args!($($arg)*))
    };
}
pub(crate) use log;

pub(crate) fn print(args: std::fmt::Arguments) {
    println!("[{}] {args}", timestamp());
}

fn timestamp() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = since_epoch.as_secs();
    let (year, month, day) = civil_from_days((seconds / 86_400) as i64);
    let time_of_day = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}Z",
        time_of_day / 3600,
        time_of_day / 60 % 60,
        time_of_day % 60,
    )
}

/// Proleptic Gregorian date for a count of days since 1970-01-01
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153; // March-based month, [0, 11]
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = year_of_era as i64 + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29)); // leap day
        assert_eq!(civil_from_days(20_454), (2026, 1, 1));
    }
}
