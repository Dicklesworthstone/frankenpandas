//! Fresh-frame-per-iteration bench: what would eager block consolidation cost
//! the ORDINARY op surface? (br-frankenpandas-uza04)
//!
//! This exists to test ONE named retry predicate, not to make a perf claim.
//! `docs/NEGATIVE_EVIDENCE.md` (2026-07-25) rejected wiring column-major block
//! consolidation into `DataFrame::new_with_axes`, because that adds a full
//! column-major copy of the frame's data to EVERY homogeneous-f64 frame produced
//! anywhere in the library (~733 construction sites), and closed with:
//!
//! > Retry predicate: ... (1) a `df_to_numpy`/`df_values` bench exists that
//! > constructs a FRESH frame per iteration (the current `df_values` bench
//! > reuses one frame and would report arm 1's cache as a win it has not
//! > earned), and (2) an A/A null control on the target worker establishes the
//! > floor. Do NOT re-propose eager consolidation in `new_with_axes` without
//! > first showing, on that same fresh-frame bench, that the ~733-site copy tax
//! > is under the null floor for the ordinary op surface.
//!
//! `examples/bench_to_numpy.rs` is exactly the reusing bench that objection
//! names: it calls `build(n)` once and loops `to_numpy()`, so a cached block
//! would look free. Here every timed slot builds its own frame.
//!
//! WHAT IS MEASURED, and why this shape can answer the question:
//!   ORDINARY    - build a fresh columnar frame the way every op builds one now
//!   CONSOLIDATED- build the same frame AND its column-major block, i.e. exactly
//!                 what eager consolidation would add at construction
//!   A/A NULL    - two ORDINARY arms, interleaved identically, which fixes the
//!                 floor: any tax smaller than this is not measurable here
//!
//! Arms interleave ABBA within each round, so drift and foreign load on a shared
//! host hit both arms alike (the same reason the sanctioned vs-pandas harness
//! uses a balanced square). The verdict is the MEDIAN of per-round ratios; a
//! mean would let one descheduled slot decide it.
//!
//! Neither arm touches `.values`/`to_numpy`. That is deliberate: the predicate
//! asks what consolidation costs the ops that never wanted a block, which is the
//! whole basis of the rejection.
//!
//! Run (needs the feature for the block constructor):
//!   env -u CARGO_TARGET_DIR cargo run -p fp-frame --release \
//!     --features block-storage --example bench_fresh_frame_consolidation_tax
//!
//! Without `--features block-storage` it refuses to run rather than silently
//! measuring the ordinary arm twice and reporting a tax of 1.000x.

use std::{collections::BTreeMap, fmt::Write as _, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use sha2::{Digest, Sha256};

/// SELF-reported binary identity, in the canonical shape the perf ledger's
/// preflight gate requires (`bench_elf_sha256=<sha> (<bytes> bytes) <abs path>`).
///
/// The point is that the PROCESS reports what it is. A sha computed afterwards
/// by `sha256sum` proves only that some file on disk had that digest, not that
/// the numbers below came out of it — which is the whole reason the gate insists
/// on this marker.
fn self_identity() -> String {
    let Ok(path) = std::env::current_exe() else {
        return "unavailable".to_owned();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return "unavailable".to_owned();
    };
    let digest = Sha256::digest(&bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    format!("{hex} ({} bytes) {}", bytes.len(), path.display())
}

/// Deterministic payload; no `rand`, so a rerun on another host is comparable.
fn column_values(rows: usize, col: usize) -> Vec<f64> {
    let salt = (col as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (0..rows)
        .map(|row| {
            let mixed = (row as u64).wrapping_mul(0x2545_F491_4F6C_DD1D) ^ salt;
            ((mixed >> 11) % 1_000_003) as f64 * 0.125
        })
        .collect()
}

fn column_names(cols: usize) -> Vec<String> {
    (0..cols).map(|col| format!("f{col}")).collect()
}

/// One ORDINARY construction: typed columns into the columnar store, exactly
/// what `new_with_column_order` does for every op output today.
fn build_columnar(rows: usize, cols: usize) -> DataFrame {
    let index = Index::new_known_unique_int64_unit_range(0, rows);
    let mut columns = BTreeMap::new();
    for (col, name) in column_names(cols).into_iter().enumerate() {
        columns.insert(name, Column::from_f64_values(column_values(rows, col)));
    }
    let order = column_names(cols);
    DataFrame::new_with_column_order(index, columns, order).expect("columnar frame")
}

/// One CONSOLIDATED construction: the same data, plus the column-major block
/// that eager consolidation would have to materialize at construction time.
/// `block[col * rows + row]` is the layout `from_f64_block_columns` documents.
#[cfg(feature = "block-storage")]
fn build_consolidated(rows: usize, cols: usize) -> DataFrame {
    let index = Index::new_known_unique_int64_unit_range(0, rows);
    let mut block = Vec::with_capacity(rows * cols);
    for col in 0..cols {
        block.extend_from_slice(&column_values(rows, col));
    }
    DataFrame::from_f64_block_columns(index, column_names(cols), block).expect("block frame")
}

/// Time one fresh-frame slot. The frame is consumed by a cheap metadata read so
/// the optimizer cannot delete the construction, and is dropped inside the timed
/// region because an op pays for its output frame's teardown too.
fn timed_slot<F: Fn(usize, usize) -> DataFrame>(build: &F, rows: usize, cols: usize) -> f64 {
    let start = Instant::now();
    let frame = build(rows, cols);
    let shape = black_box(frame.shape());
    black_box(shape.0 + shape.1);
    drop(frame);
    start.elapsed().as_secs_f64() * 1e6
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|left, right| left.partial_cmp(right).expect("no NaN timings"));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// ⚠️ SMALL SIZES ARE ALLOCATOR-STATE DEPENDENT — DO NOT QUOTE A 10k TAX.
///
/// At 10k x 10 (0.8 MB) this bench has produced BOTH ~2.1x and ~0.97x for the
/// same source, on the same host, minutes apart. The variable is what the
/// allocator already holds: the CONSOLIDATED arm wants one contiguous 0.8 MB
/// buffer per iteration, so whether that comes from a warm free block or a
/// fresh mmap (page faults, `clear_page_erms`) decides the arm. Merely adding a
/// startup self-hash — which reads the executable into a large `Vec<u8>` and
/// drops it — was enough to move the 10k number by 2x, while leaving 100k
/// untouched. Running `100k` before `10k` is likewise a different measurement
/// from running `10k` alone.
///
/// 100k x 10 (8 MB) is stable across every binary and ordering tried, because
/// there the 8 MB copy dominates any allocator bookkeeping. Rest conclusions on
/// that size; treat the small size as context only.
#[cfg(feature = "block-storage")]
fn run_size(rows: usize, cols: usize, rounds: usize) {
    // Warm the allocator and any one-time init so round 0 is not an outlier
    // that the median then has to survive.
    for _ in 0..3 {
        black_box(timed_slot(&build_columnar, rows, cols));
        black_box(timed_slot(&build_consolidated, rows, cols));
    }

    let mut effect_ratios = Vec::with_capacity(rounds);
    let mut null_ratios = Vec::with_capacity(rounds);
    let mut ordinary_us = Vec::with_capacity(rounds);
    let mut consolidated_us = Vec::with_capacity(rounds);

    for _ in 0..rounds {
        // EFFECT round, ABBA: ordinary, consolidated, consolidated, ordinary.
        let a0 = timed_slot(&build_columnar, rows, cols);
        let b0 = timed_slot(&build_consolidated, rows, cols);
        let b1 = timed_slot(&build_consolidated, rows, cols);
        let a1 = timed_slot(&build_columnar, rows, cols);
        let ordinary = (a0 + a1) / 2.0;
        let consolidated = (b0 + b1) / 2.0;
        ordinary_us.push(ordinary);
        consolidated_us.push(consolidated);
        effect_ratios.push(consolidated / ordinary);

        // NULL round, same ABBA shape but ORDINARY in every slot. This is the
        // floor: whatever spread this shows is what the host alone produces.
        let n0 = timed_slot(&build_columnar, rows, cols);
        let n1 = timed_slot(&build_columnar, rows, cols);
        let n2 = timed_slot(&build_columnar, rows, cols);
        let n3 = timed_slot(&build_columnar, rows, cols);
        null_ratios.push(((n1 + n2) / 2.0) / ((n0 + n3) / 2.0));
    }

    let effect = median(effect_ratios);
    let null = median(null_ratios);
    let null_deviation = (null - 1.0).abs();
    let effect_deviation = (effect - 1.0).abs();
    // The predicate's own words: the tax must be UNDER the null floor. Anything
    // at or above the floor is a measurable cost on the ordinary op surface.
    let predicate_met = effect_deviation < null_deviation;

    println!(
        "SIZE rows={rows} cols={cols} bytes={} rounds={rounds}",
        rows * cols * 8
    );
    println!(
        "  ordinary_p50_us={:.3} consolidated_p50_us={:.3} added_us={:.3}",
        median(ordinary_us.clone()),
        median(consolidated_us.clone()),
        median(consolidated_us) - median(ordinary_us)
    );
    println!(
        "  TAX_median={effect:.6} (deviation {effect_deviation:.6})  \
         AA_NULL_median={null:.6} (floor {null_deviation:.6})"
    );
    // State the comparison neutrally. An earlier draft printed "(tax is under
    // the null floor: X < Y)" unconditionally, which reads as an assertion even
    // when the verdict is false — exactly the sentence a skimming reader would
    // quote back as evidence for the opposite conclusion.
    println!(
        "  PREDICATE_MET={predicate_met}  \
         (predicate needs tax_deviation < null_floor; measured {effect_deviation:.6} vs \
         {null_deviation:.6}, i.e. {:.1}x the floor)",
        if null_deviation > 0.0 {
            effect_deviation / null_deviation
        } else {
            f64::INFINITY
        }
    );
}

#[cfg(not(feature = "block-storage"))]
fn main() {
    eprintln!(
        "bench_fresh_frame_consolidation_tax requires --features block-storage \
         (DataFrame::from_f64_block_columns is behind it). Refusing to run: without \
         it both arms would be the ordinary path and the tax would read as 1.000x."
    );
    std::process::exit(2);
}

#[cfg(feature = "block-storage")]
fn main() {
    let rounds: usize = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(15);

    println!("bench_elf_sha256={}", self_identity());
    println!("bench_fresh_frame_consolidation_tax (br-frankenpandas-uza04)");
    println!("fresh frame per timed slot; ABBA interleave; median of per-round ratios");
    println!("TAX = consolidated / ordinary; AA_NULL = ordinary / ordinary");

    // The ledger's own worked example is a 100k x 10 f64 frame (8 MB), so that
    // size is the one the predicate is really about. 10k is carried alongside
    // because a fixed per-construction cost would show up there first.
    //
    // The size list is selectable because the ORDER matters at small sizes: see
    // the small-size warning in `run_size`. `10k` alone and `10k` after `100k`
    // are different measurements, and being able to run each is how that was
    // established rather than guessed.
    let sizes: Vec<usize> = match std::env::args().nth(2) {
        Some(list) => list
            .split(',')
            .filter_map(|entry| entry.trim().parse().ok())
            .collect(),
        None => vec![10_000, 100_000],
    };
    for rows in sizes {
        run_size(rows, 10, rounds);
    }
}
