//! Adjudication coverage for the 2026-06-20 "UNVERIFIED CODE-ONLY STACK"
//! (`docs/NEGATIVE_EVIDENCE.md:36`) — the `dt.*` typed Datetime64 fast paths that
//! were written during a disk-CRITICAL window and, per that runbook, were never
//! compiled or verified.
//!
//! **Why this file exists.** Every `dt.*` fast path is gated on
//! `Column::as_datetime64_slice()` returning `Some` (a typed Datetime64 backing)
//! with no `NAT`. Every pre-existing `dt_*` unit test in `fp-frame/src/lib.rs`
//! builds its Series from `Scalar::Utf8` date *strings*, so the column is `Utf8`,
//! the gate returns `None`, and the generic chrono path runs instead. Those tests
//! are green, but they were green *without ever executing the typed code under
//! test* — the same "the bench never ran the candidate" failure mode the campaign
//! ledger-resurrection audit exists to catch, applied to tests.
//!
//! This file closes that hole: it drives BOTH paths over identical instants and
//! asserts the typed fast path is bit-identical to the generic path, accessor by
//! accessor. `typed_datetime64_backing_is_actually_reached` is the reachability
//! proof — without it, a green differential would again prove nothing.

use fp_frame::{Series, to_datetime};
use fp_index::IndexLabel;
use fp_types::Scalar;

/// Instants chosen to exercise the calendar edges the typed civil-date helpers
/// re-derive by hand: leap day, year/quarter/month boundaries, a pre-epoch
/// (negative-nanos) instant, and a non-midnight time-of-day.
const INSTANTS: &[&str] = &[
    "2024-01-01T00:00:00", // leap year, year/quarter/month start, midnight
    "2024-02-29T23:59:59", // leap day, month end, last second of the day
    "2023-12-31T12:00:00", // non-leap year end, quarter end
    "2024-07-04T06:30:15", // mid-year, arbitrary time-of-day
    "2024-03-31T23:59:59", // quarter end (Q1), month end
    "2024-04-01T00:00:00", // quarter start (Q2), month start
    "1969-07-20T20:17:40", // PRE-EPOCH: negative nanos, exercises div_euclid
    "2000-02-29T01:02:03", // century leap year (divisible by 400)
    "1900-03-01T00:00:00", // day after a NON-leap century year (div by 100)
];

fn utf8_series() -> Series {
    Series::from_values(
        "d",
        (0..INSTANTS.len() as i64).map(IndexLabel::Int64).collect(),
        INSTANTS
            .iter()
            .map(|s| Scalar::Utf8((*s).to_string()))
            .collect(),
    )
    .unwrap()
}

/// The Utf8 series converted to a real Datetime64 column — the shape that
/// actually engages the typed fast paths.
fn datetime64_series() -> Series {
    to_datetime(&utf8_series()).expect("to_datetime over ISO-8601 strings")
}

#[test]
fn typed_datetime64_backing_is_actually_reached() {
    // REACHABILITY PROOF. If this regresses to None, every differential below
    // silently degrades into "generic path == generic path" and stops guarding
    // the typed code. Assert the exact gate the fast paths use.
    let dt64 = datetime64_series();
    let nanos = dt64
        .column()
        .as_datetime64_slice()
        .expect("to_datetime must yield a TYPED Datetime64 backing, else the dt fast paths are dead code in tests");
    assert_eq!(nanos.len(), INSTANTS.len());
    assert!(
        !nanos.contains(&fp_types::Timestamp::NAT),
        "fixture must be all-valid: a NAT disables every typed dt fast path"
    );

    // And confirm the control arm really is the generic path (Utf8-backed).
    assert!(
        utf8_series().column().as_datetime64_slice().is_none(),
        "the Utf8 control arm must NOT have a Datetime64 backing"
    );
}

/// Assert a `dt` accessor agrees between the generic (Utf8) and typed
/// (Datetime64) paths.
macro_rules! assert_dt_paths_agree {
    ($method:ident) => {{
        let generic = utf8_series().dt().$method().expect(concat!(
            "dt.",
            stringify!($method),
            " on the Utf8 path"
        ));
        let typed = datetime64_series().dt().$method().expect(concat!(
            "dt.",
            stringify!($method),
            " on the Datetime64 path"
        ));
        assert_eq!(
            generic.values(),
            typed.values(),
            concat!(
                "dt.",
                stringify!($method),
                ": typed Datetime64 fast path DIVERGES from the generic path"
            )
        );
    }};
}

#[test]
fn dt_numeric_components_match_generic_path() {
    assert_dt_paths_agree!(year);
    assert_dt_paths_agree!(month);
    assert_dt_paths_agree!(day);
    assert_dt_paths_agree!(hour);
    assert_dt_paths_agree!(minute);
    assert_dt_paths_agree!(second);
    assert_dt_paths_agree!(microsecond);
    assert_dt_paths_agree!(nanosecond);
    assert_dt_paths_agree!(quarter);
}

#[test]
fn dt_calendar_components_match_generic_path() {
    assert_dt_paths_agree!(dayofweek);
    assert_dt_paths_agree!(dayofyear);
    assert_dt_paths_agree!(weekofyear);
    assert_dt_paths_agree!(days_in_month);
}

#[test]
fn dt_boolean_predicates_match_generic_path() {
    assert_dt_paths_agree!(is_month_start);
    assert_dt_paths_agree!(is_month_end);
    assert_dt_paths_agree!(is_quarter_start);
    assert_dt_paths_agree!(is_quarter_end);
    assert_dt_paths_agree!(is_year_start);
    assert_dt_paths_agree!(is_year_end);
    assert_dt_paths_agree!(is_leap_year);
}

#[test]
fn dt_string_and_struct_components_match_generic_path() {
    assert_dt_paths_agree!(month_name);
    assert_dt_paths_agree!(day_name);
    assert_dt_paths_agree!(date);
    assert_dt_paths_agree!(time);
}

/// Ground truth from pandas 2.2.3 for [`INSTANTS`], captured 2026-07-25:
/// `pd.to_datetime(pd.Series(INSTANTS)).dt.day_name()` / `.dt.dayofweek`.
///
/// A typed-vs-generic differential cannot catch a bug BOTH paths share, so the
/// weekday family — the one place this stack was found to diverge — is also
/// pinned against the oracle.
const PANDAS_DAY_NAME: &[&str] = &[
    "Monday",   // 2024-01-01
    "Thursday", // 2024-02-29
    "Sunday",   // 2023-12-31
    "Thursday", // 2024-07-04
    "Sunday",   // 2024-03-31
    "Monday",   // 2024-04-01
    "Sunday",   // 1969-07-20 (pre-epoch; Apollo 11 landing was a Sunday)
    "Tuesday",  // 2000-02-29
    "Thursday", // 1900-03-01
];
const PANDAS_DAYOFWEEK: &[i64] = &[0, 3, 6, 3, 6, 0, 6, 1, 3];

#[test]
fn dt_weekday_family_matches_pandas_oracle() {
    for series in [utf8_series(), datetime64_series()] {
        let names = series.dt().day_name().expect("day_name");
        let expected: Vec<Scalar> = PANDAS_DAY_NAME
            .iter()
            .map(|s| Scalar::Utf8((*s).to_string()))
            .collect();
        assert_eq!(names.values(), expected.as_slice(), "dt.day_name vs pandas");

        let dow = series.dt().dayofweek().expect("dayofweek");
        let expected: Vec<Scalar> = PANDAS_DAYOFWEEK
            .iter()
            .copied()
            .map(Scalar::Int64)
            .collect();
        assert_eq!(dow.values(), expected.as_slice(), "dt.dayofweek vs pandas");
    }
}

#[test]
fn dt_strftime_matches_pandas_oracle() {
    // A typed-vs-generic differential cannot see a bug both arms share, and both
    // strftime arms derive the date from the SAME formula, so pin it to the
    // oracle independently. `Timestamp::strftime` mixes truncating `total_secs /
    // 86400` with a positive-wrapped `secs_of_day`; that pairing was checked
    // against pandas here and is CORRECT for the pre-1970 non-midnight case
    // (unlike `day_name`, which was not). This is the regression guard for it.
    let expected_dates = [
        "2024-01-01",
        "2024-02-29",
        "2023-12-31",
        "2024-07-04",
        "2024-03-31",
        "2024-04-01",
        "1969-07-20",
        "2000-02-29",
        "1900-03-01",
    ];
    for series in [utf8_series(), datetime64_series()] {
        let got = series.dt().strftime("%Y-%m-%d").expect("strftime");
        let want: Vec<Scalar> = expected_dates
            .iter()
            .map(|s| Scalar::Utf8((*s).to_string()))
            .collect();
        assert_eq!(
            got.values(),
            want.as_slice(),
            "dt.strftime(%Y-%m-%d) vs pandas"
        );
    }
}

#[test]
fn dt_strftime_matches_generic_path() {
    // strftime takes a format argument, so it cannot use the macro above.
    // Cover a format touching every field the typed emitter hand-rolls.
    for fmt in ["%Y-%m-%d", "%H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%B %A %j"] {
        let generic = utf8_series().dt().strftime(fmt).expect("strftime Utf8");
        let typed = datetime64_series()
            .dt()
            .strftime(fmt)
            .expect("strftime Datetime64");
        assert_eq!(
            generic.values(),
            typed.values(),
            "dt.strftime({fmt:?}): typed fast path diverges from the generic path"
        );
    }
}
