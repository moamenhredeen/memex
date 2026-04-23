//! Note ID generator: `YYYYMMDDTHHMMSS-XXXX` in UTC + 4 base36 chars.
//!
//! Properties we care about:
//! - **Sortable by creation time** when a directory is listed externally.
//! - **Collision-free** at personal-vault scale, including fast capture
//!   and multi-device git merges.
//! - **No clock dependency** for uniqueness — the random suffix handles
//!   same-second creations.
//! - **Short enough** to read at a glance (20 chars).
//!
//! The timestamp formatting is done manually to avoid pulling in
//! `chrono`. Uses Howard Hinnant's civil-from-days algorithm.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const BASE36: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
/// 36^4 — the suffix counter wraps at this point.
const SUFFIX_MOD: u32 = 1_679_616;

/// Generate a new note ID using the current system clock in UTC.
pub fn generate() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_id(secs, &next_suffix())
}

/// Current UTC time as an ISO-8601 string (e.g. `2026-04-23T14:23:01Z`).
/// Used for the `created` frontmatter field.
pub fn iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = utc_components(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, mo, d, h, mi, s)
}

/// Format an ID from an explicit UTC unix timestamp and suffix. Exposed
/// so tests can pin the timestamp without mocking the clock.
pub fn format_id(unix_secs: u64, suffix: &str) -> String {
    let (y, mo, d, h, mi, s) = utc_components(unix_secs);
    format!("{:04}{:02}{:02}T{:02}{:02}{:02}-{}", y, mo, d, h, mi, s, suffix)
}

/// 4-char base36 suffix. Uses a process-local atomic counter seeded
/// once with randomness, so:
/// - **In-process**: no two calls collide until the counter wraps
///   (36^4 = 1 679 616 calls).
/// - **Across processes / machines**: the random seed makes aligned
///   starts unlikely, so two machines generating at the same second
///   almost never collide.
fn next_suffix() -> String {
    static COUNTER: OnceLock<AtomicU32> = OnceLock::new();
    let counter = COUNTER.get_or_init(|| AtomicU32::new(fastrand::u32(..)));
    let mut n = counter.fetch_add(1, Ordering::Relaxed) % SUFFIX_MOD;
    let mut buf = [b'0'; 4];
    for i in (0..4).rev() {
        buf[i] = BASE36[(n % 36) as usize];
        n /= 36;
    }
    // Safe — every byte is ASCII from BASE36.
    String::from_utf8(buf.to_vec()).expect("base36 produces ASCII")
}

/// Break a UTC unix timestamp into `(year, month, day, hour, minute, second)`.
///
/// Months are 1..=12, days 1..=31. Uses Howard Hinnant's civil-from-days
/// algorithm, which is correct for all years in a reasonable range
/// (tested to ±2^30 days from epoch).
fn utc_components(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let time_of_day = secs % 86_400;
    let hour = (time_of_day / 3600) as u32;
    let minute = ((time_of_day % 3600) / 60) as u32;
    let second = (time_of_day % 60) as u32;

    // Shift so day 0 is 0000-03-01 (Hinnant's era reference).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // day of era, 0..=146_096
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // 0..=399
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // 0..=365
    let mp = (5 * doy + 2) / 153; // 0..=11
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // 1..=31
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // 1..=12
    let year = (y + if m <= 2 { 1 } else { 0 }) as u32;
    (year, m, d, hour, minute, second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn format_matches_schema() {
        let id = format_id(0, "abcd");
        assert_eq!(id, "19700101T000000-abcd");
    }

    #[test]
    fn format_known_timestamps() {
        // 2026-04-23T14:23:01Z → 1745418181 seconds since epoch.
        let id = format_id(1745418181, "k3n8");
        assert_eq!(id, "20250423T142301-k3n8");
        // Y2K + 1 second:
        assert_eq!(format_id(946_684_801, "zzzz"), "20000101T000001-zzzz");
        // 2038 rollover boundary:
        assert_eq!(format_id(2_147_483_647, "0000"), "20380119T031407-0000");
    }

    #[test]
    fn utc_components_spot_check() {
        // 2026-04-23T14:23:01Z
        let secs = 1_745_418_181;
        let (y, mo, d, h, mi, s) = utc_components(secs);
        assert_eq!((y, mo, d, h, mi, s), (2025, 4, 23, 14, 23, 1));
    }

    #[test]
    fn utc_components_handles_leap_year() {
        // 2024-02-29T12:00:00Z — leap day
        let secs = 1_709_208_000;
        let (y, mo, d, h, mi, s) = utc_components(secs);
        assert_eq!((y, mo, d, h, mi, s), (2024, 2, 29, 12, 0, 0));
    }

    #[test]
    fn generate_produces_valid_format() {
        let id = generate();
        assert_eq!(id.len(), 20);
        assert_eq!(id.chars().nth(8), Some('T'));
        assert_eq!(id.chars().nth(15), Some('-'));
        // All timestamp digits
        for i in [0, 1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14] {
            assert!(id.chars().nth(i).unwrap().is_ascii_digit(), "pos {} not digit", i);
        }
        // Suffix is base36 lowercase
        for i in 16..20 {
            let c = id.chars().nth(i).unwrap();
            assert!(c.is_ascii_digit() || ('a'..='z').contains(&c));
        }
    }

    #[test]
    fn generate_is_collision_free_across_1k_calls() {
        // At 4-char base36 = 36^4 = 1.68M possibilities, 1000 calls
        // have a birthday-collision probability of ~30% *if* the
        // timestamp doesn't change. In practice the suffix + second
        // granularity make it effectively zero in tests.
        let ids: HashSet<String> = (0..1000).map(|_| generate()).collect();
        assert_eq!(ids.len(), 1000);
    }
}
