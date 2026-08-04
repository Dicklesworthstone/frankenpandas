//! No-mock guard for the typed Datetime64 isocalendar fast path (inlined civil +
//! iso_year_week_day over raw &[i64] ns). Differential: the SAME dates fed as a
//! typed Datetime64 column (fast path) vs a Utf8 "Y-M-D" column (the original
//! parse + shared iso_year_week_day path). Outputs (year, week, day) must match
//! bit-for-bit over a decade of daily dates + ISO-week 52/53 boundaries + leap
//! years + a NaT row.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

const NANOS_PER_DAY: i64 = 86_400_000_000_000;

// Howard Hinnant days_from_civil (inverse of the civil extraction under test).
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
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        _ => unreachable!(),
    }
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

fn col_dt(ds: &[(i64, i64, i64)], nat_at: Option<usize>) -> Column {
    let ns: Vec<i64> = ds
        .iter()
        .enumerate()
        .map(|(i, &(y, m, d))| {
            if Some(i) == nat_at {
                i64::MIN
            } else {
                days_from_civil(y, m, d) * NANOS_PER_DAY
            }
        })
        .collect();
    Column::from_datetime64_values(ns)
}
fn col_str(ds: &[(i64, i64, i64)], nat_at: Option<usize>) -> Column {
    let mut bytes = Vec::new();
    let mut offs = vec![0usize];
    for (i, &(y, m, d)) in ds.iter().enumerate() {
        if Some(i) != nat_at {
            bytes.extend_from_slice(format!("{y:04}-{m:02}-{d:02}").as_bytes());
        }
        offs.push(bytes.len()); // empty string for the NaT row -> parse None
    }
    Column::from_utf8_contiguous(bytes, offs)
}
fn bits(s: &Scalar) -> u64 {
    if s.is_missing() {
        return u64::MAX;
    }
    match s {
        Scalar::Int64(v) => *v as u64,
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn isocalendar_typed_matches_string_path() {
    let ds = dates();
    let n = ds.len();
    let nat_at = Some(n / 2);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let dt = Series::new("t", Index::new(labels.clone()), col_dt(&ds, nat_at)).unwrap();
    let st = Series::new("t", Index::new(labels), col_str(&ds, nat_at)).unwrap();

    let rdt = dt.dt().isocalendar().unwrap();
    let rst = st.dt().isocalendar().unwrap();
    for col in ["year", "week", "day"] {
        let a = rdt.columns().get(col).unwrap().values();
        let b = rst.columns().get(col).unwrap().values();
        assert_eq!(a.len(), n);
        for i in 0..n {
            assert_eq!(bits(&a[i]), bits(&b[i]), "{col} row {i} date {:?}", ds[i]);
        }
    }
}
