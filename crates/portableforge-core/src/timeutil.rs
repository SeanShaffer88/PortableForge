//! Timezone-safe timestamps (program-100 item 80).
//!
//! Everything PortableForge writes down — session records, session
//! log lines, build reports — carries a UTC ISO-8601 stamp
//! (`2026-10-24T18:05:00Z`), never a local time whose zone the reader
//! would have to guess. No date library: the civil-from-days
//! conversion below is exact for any realistic timestamp.

use std::time::{SystemTime, UNIX_EPOCH};

/// (year, month, day) for a count of days since 1970-01-01
/// (Howard Hinnant's civil-from-days algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn parts(t: SystemTime) -> (i64, u32, u32, u32, u32, u32) {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        (tod / 3600) as u32,
        ((tod % 3600) / 60) as u32,
        (tod % 60) as u32,
    )
}

/// UTC ISO-8601, e.g. `2026-10-24T18:05:00Z`.
pub fn iso_utc(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = parts(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Folder-safe UTC stamp, e.g. `20261024-180500` (session ids).
pub fn compact_utc(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = parts(t);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// The current time as a UTC ISO-8601 stamp.
pub fn now_iso() -> String {
    iso_utc(SystemTime::now())
}

/// The current time as a folder-safe UTC stamp.
pub fn now_compact() -> String {
    compact_utc(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn known_instants_format_correctly() {
        assert_eq!(iso_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        assert_eq!(
            iso_utc(UNIX_EPOCH + Duration::from_secs(1_735_689_600)),
            "2025-01-01T00:00:00Z"
        );
        assert_eq!(
            iso_utc(UNIX_EPOCH + Duration::from_secs(1_735_689_600 + 3_723)),
            "2025-01-01T01:02:03Z"
        );
        // Leap day: 2024-02-29.
        assert_eq!(
            iso_utc(UNIX_EPOCH + Duration::from_secs(1_709_164_800)),
            "2024-02-29T00:00:00Z"
        );
        assert_eq!(
            compact_utc(UNIX_EPOCH + Duration::from_secs(1_735_689_600)),
            "20250101-000000"
        );
    }
}
