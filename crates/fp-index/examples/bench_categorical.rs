//! Bench + golden digest for CategoricalIndex category-membership ops.
//!
//! Run: cargo run -p fp-index --example bench_categorical --release
//!
//! Prints a stable GOLDEN digest (semantics) and timing for the O(n·k)
//! membership hot paths (with_categories / from_values / set_categories /
//! add_categories / remove_categories).

use std::time::Instant;

use fp_index::CategoricalIndex;

fn fmt_err<T>(r: &Result<T, fp_index::IndexError>) -> String {
    match r {
        Ok(_) => "OK".to_string(),
        Err(e) => format!("ERR:{e}"),
    }
}

fn golden() -> String {
    // Small deterministic battery exercising ok + error paths.
    let mut out = String::new();
    let cats: Vec<String> = (0..8).map(|i| format!("c{i}")).collect();
    let labels: Vec<String> = (0..40).map(|i| format!("c{}", i % 8)).collect();

    // with_categories: ok
    let ci = CategoricalIndex::with_categories(labels.clone(), cats.clone(), false).unwrap();
    out.push_str(&format!("wc_ok cats={:?}\n", ci.categories()));
    // with_categories: error (label not present)
    let bad =
        CategoricalIndex::with_categories(vec!["zzz".into(), "c1".into()], cats.clone(), false);
    out.push_str(&format!("wc_err {}\n", fmt_err(&bad)));

    // from_values: first-seen categories order
    let fv = CategoricalIndex::from_values(
        vec!["b".into(), "a".into(), "b".into(), "c".into(), "a".into()],
        true,
    );
    out.push_str(&format!(
        "fv cats={:?} ordered={}\n",
        fv.categories(),
        fv.ordered()
    ));

    // set_categories: ok (superset)
    let mut sc_cats = cats.clone();
    sc_cats.push("c8".into());
    out.push_str(&format!("sc_ok {}\n", fmt_err(&ci.set_categories(sc_cats))));
    // set_categories: error (drops c0 which is in use)
    let drop_cats: Vec<String> = (1..8).map(|i| format!("c{i}")).collect();
    out.push_str(&format!(
        "sc_err {}\n",
        fmt_err(&ci.set_categories(drop_cats))
    ));

    // add_categories: ok + error(already present)
    out.push_str(&format!(
        "add_ok {}\n",
        fmt_err(&ci.add_categories(vec!["c8".into(), "c9".into()]))
    ));
    out.push_str(&format!(
        "add_err {}\n",
        fmt_err(&ci.add_categories(vec!["c2".into()]))
    ));

    // remove_categories: ok (unused) + error(in use) + error(not a category)
    let ci2 =
        CategoricalIndex::with_categories(labels.clone(), sc_cats_for_remove(), false).unwrap();
    out.push_str(&format!(
        "rm_ok {}\n",
        fmt_err(&ci2.remove_categories(&["c8".into()]))
    ));
    out.push_str(&format!(
        "rm_err_inuse {}\n",
        fmt_err(&ci2.remove_categories(&["c0".into()]))
    ));
    out.push_str(&format!(
        "rm_err_nocat {}\n",
        fmt_err(&ci2.remove_categories(&["nope".into()]))
    ));
    out
}

fn sc_cats_for_remove() -> Vec<String> {
    let mut v: Vec<String> = (0..8).map(|i| format!("c{i}")).collect();
    v.push("c8".into()); // unused extra category
    v
}

fn main() {
    // ---- GOLDEN ----
    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    // ---- TIMING ----
    // N labels drawn from K categories: with_categories validates every label
    // against the K-category set (current code: O(N·K) Vec::contains).
    let k: usize = 4000;
    let n: usize = 400_000;
    let cats: Vec<String> = (0..k).map(|i| format!("cat{i}")).collect();
    let labels: Vec<String> = (0..n).map(|i| format!("cat{}", i % k)).collect();

    // with_categories (validate N labels vs K cats)
    let t = Instant::now();
    let ci = CategoricalIndex::with_categories(labels.clone(), cats.clone(), false).unwrap();
    let d_wc = t.elapsed();

    // from_values (dedup N labels -> K cats)
    let t = Instant::now();
    let fv = CategoricalIndex::from_values(labels.clone(), false);
    let d_fv = t.elapsed();
    assert_eq!(fv.categories().len(), k);

    // set_categories (validate N labels vs K' cats); superset so it succeeds
    let mut cats2 = cats.clone();
    cats2.push("extra".into());
    let t = Instant::now();
    let _ = ci.set_categories(cats2).unwrap();
    let d_sc = t.elapsed();

    // add_categories (K new cats checked vs K existing)
    let new_cats: Vec<String> = (k..2 * k).map(|i| format!("cat{i}")).collect();
    let t = Instant::now();
    let _ = ci.add_categories(new_cats).unwrap();
    let d_add = t.elapsed();

    println!(
        "TIMING n={n} k={k} with_categories={:.3}ms from_values={:.3}ms set_categories={:.3}ms add_categories={:.3}ms",
        d_wc.as_secs_f64() * 1e3,
        d_fv.as_secs_f64() * 1e3,
        d_sc.as_secs_f64() * 1e3,
        d_add.as_secs_f64() * 1e3,
    );
}
