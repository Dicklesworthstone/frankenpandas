//! Guard for to_period's numeric civil label fast path (period_label_numeric over
//! a Datetime64 index) vs the chrono path (a Utf8 datetime-string index, which
//! routes through parse_naive_datetime_value + format_period_label). The two must
//! produce identical period-label indexes across M/D/Y/Q/W over a decade of dates
//! incl. ISO/calendar boundaries.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};

const NANOS_PER_DAY: i64 = 86_400_000_000_000;

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}
fn dim(y: i64, m: i64) -> i64 {
    [
        31,
        if is_leap(y) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ][(m - 1) as usize]
}
fn dates() -> Vec<(i64, i64, i64)> {
    let mut v = Vec::new();
    for y in 2015..=2026 {
        for m in 1..=12 {
            for d in 1..=dim(y, m) {
                v.push((y, m, d));
            }
        }
    }
    v
}

fn dt_series(ds: &[(i64, i64, i64)]) -> Series {
    let labels: Vec<IndexLabel> = ds
        .iter()
        .map(|&(y, m, d)| IndexLabel::Datetime64(days_from_civil(y, m, d) * NANOS_PER_DAY))
        .collect();
    Series::new("t", Index::new(labels), col(ds.len())).unwrap()
}
fn str_series(ds: &[(i64, i64, i64)]) -> Series {
    let labels: Vec<IndexLabel> = ds
        .iter()
        .map(|&(y, m, d)| IndexLabel::Utf8(format!("{y:04}-{m:02}-{d:02}T00:00:00")))
        .collect();
    Series::new("t", Index::new(labels), col(ds.len())).unwrap()
}
fn col(n: usize) -> Column {
    Column::from_f64_values((0..n).map(|i| i as f64).collect())
}

#[test]
fn to_period_numeric_matches_chrono_path() {
    let ds = dates();
    for freq in ["M", "D", "Y", "Q", "W"] {
        let a = dt_series(&ds).to_period(freq).unwrap();
        let b = str_series(&ds).to_period(freq).unwrap();
        let la = a.index().labels();
        let lb = b.index().labels();
        assert_eq!(la.len(), lb.len(), "freq {freq} len");
        for i in 0..la.len() {
            assert_eq!(la[i], lb[i], "freq {freq} row {i} date {:?}", ds[i]);
        }
    }
}
