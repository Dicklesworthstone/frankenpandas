//! value_counts_with_options Float64-numeric-index golden (br-frankenpandas-8sxez).
//! The basic `value_counts()` path already emits a numeric Float64 index (peer
//! work); this completes the parity by routing the with-options path
//! (normalize / sort / ascending / dropna) through the same float-aware label
//! mapper. Prints the (index-label, value) output for a fixed float Series under
//! several option combos so it can be diffed against the pandas 2.2.3 oracle.
//!
//! Run: cargo run --profile release-perf -p fp-frame --example vc_float_golden

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;

fn float_series(data: Vec<f64>) -> Series {
    let n = data.len();
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let col = Column::from_f64_values(data);
    Series::new("x".to_string(), index, col).expect("series")
}

fn dump(tag: &str, out: &Series) {
    print!("{tag}: ");
    for (label, value) in out.index().labels().iter().zip(out.column().values()) {
        print!("{label}={value:?} ");
    }
    println!("| first_label_kind={:?}", out.index().labels().first());
}

fn main() {
    let data = vec![1.0, 2.0, 2.0, 3.5, 3.5, 3.5];
    let s = float_series(data);

    dump(
        "normalize",
        &s.value_counts_with_options(true, true, false, true)
            .expect("vc"),
    );
    dump(
        "dropna_false",
        &s.value_counts_with_options(false, true, false, false)
            .expect("vc"),
    );
    dump(
        "sort_false",
        &s.value_counts_with_options(false, false, false, true)
            .expect("vc"),
    );
    dump(
        "ascending",
        &s.value_counts_with_options(false, true, true, true)
            .expect("vc"),
    );
}
