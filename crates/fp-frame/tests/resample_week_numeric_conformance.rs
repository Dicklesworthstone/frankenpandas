//! Guard for the numeric week-ending-Sunday ordinal used by resample('W') after
//! replacing the per-row chrono NaiveDate path. The numeric formula
//! (dse = ns / NANOS_PER_DAY; weekday-from-Sunday = (dse+4) mod 7;
//!  ord = dse + days_to_sunday + 719_163) must equal chrono's
//! date.weekday() + checked_add + num_days_from_ce VERBATIM, including pre-1970
//! (negative ns) and year boundaries.

use chrono::{Datelike, Duration, NaiveDate};

const NANOS_PER_DAY: i64 = 86_400_000_000_000;

fn numeric_sunday_ord(ns: i64) -> i64 {
    let dse = ns.div_euclid(NANOS_PER_DAY);
    let wd_from_sun = (dse + 4).rem_euclid(7);
    let days_to_sunday = (7 - wd_from_sun) % 7;
    dse + days_to_sunday + 719_163
}

fn chrono_sunday_ord(ns: i64) -> i64 {
    let dse = ns.div_euclid(NANOS_PER_DAY);
    // 1970-01-01 == num_days_from_ce 719163.
    let date = NaiveDate::from_num_days_from_ce_opt((dse + 719_163) as i32).unwrap();
    let days_to_sunday = i64::from((7 - date.weekday().num_days_from_sunday()) % 7);
    let s = date
        .checked_add_signed(Duration::days(days_to_sunday))
        .unwrap();
    i64::from(s.num_days_from_ce())
}

#[test]
fn numeric_week_sunday_ord_matches_chrono() {
    let day = NANOS_PER_DAY;
    // ~80 years of daily ns spanning pre/post 1970, plus sub-day offsets.
    let mut ns = -10_957 * day; // ~1940-01-01
    let end = 20_454 * day; // ~2026
    while ns <= end {
        for off in [0i64, 1, day / 2, day - 1] {
            let v = ns + off;
            assert_eq!(
                numeric_sunday_ord(v),
                chrono_sunday_ord(v),
                "mismatch at ns={v} (dse={})",
                v.div_euclid(day)
            );
        }
        ns += day;
    }
}
