//! Copy-on-write / aliasing guard for the `block-storage` shared `Arc<[f64]>` block.
//!
//! Context: `docs/NEGATIVE_EVIDENCE.md`, 2026-07-26 QuietHarbor primitive-transfer row.
//! franken_networkx's `CachedSnapshotView` win (77,795×) came with a proof obligation —
//! they proved that refreshing one shared clone leaves its sibling stale. Sharing an
//! `Arc` without an independence proof is just aliasing. Our invariant is the mirror
//! image: **mutating one frame must never be observable through another frame sharing
//! the same block.**
//!
//! That row flagged the invariant as "unenforced". A source audit then showed something
//! stronger: `fp-frame` exposes **exactly one** `pub fn` taking `&mut self` in the whole
//! crate (`OnlineEwm::update`, an EWM accumulator). `DataFrame`/`Series` have **no
//! in-place mutation surface at all** — `with_column` / `drop_column` / `rename` all take
//! `&self` and return a new value. So the block cannot be aliased-and-mutated, because a
//! frame cannot be mutated.
//!
//! The residual risk is therefore not today's code but a *future* `&mut self` method. This
//! file is the tripwire for that: it pins the value-semantics behaviour so that adding an
//! in-place mutator which aliases a shared block fails here instead of silently corrupting
//! a sibling frame.
//!
//! Feature-gated: `block-storage` is default-OFF, so this runs in the feature lane via
//! `cargo test -p fp-frame --features block-storage --test block_storage_sharing_conformance`.

#![cfg(feature = "block-storage")]

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;

const ROWS: usize = 4;
const COLS: usize = 3;

/// Column-major block: element (row, col) is `block[col * ROWS + row]`.
fn block_frame() -> DataFrame {
    let names: Vec<String> = (0..COLS).map(|c| format!("c{c}")).collect();
    let block: Vec<f64> = (0..(ROWS * COLS)).map(|i| i as f64).collect();
    DataFrame::from_f64_block_columns(
        Index::new_known_unique_int64_unit_range(0, ROWS),
        names,
        block,
    )
    .expect("block-backed frame")
}

#[test]
fn block_backed_frame_exposes_an_o1_view() {
    // Reachability: if this stops being block-backed, every assertion below degrades
    // into testing the ordinary columnar path and stops guarding the shared block.
    let df = block_frame();
    let view = df
        .to_numpy_block_view()
        .expect("frame built by from_f64_block_columns must be block-backed");
    assert_eq!(view.rows, ROWS);
    assert_eq!(view.cols, COLS);
    assert_eq!(view.block.len(), ROWS * COLS);
    // Column-major layout contract — must match pandas' F-contiguous `.values`.
    assert_eq!(view.block[0], 0.0);
    assert_eq!(view.block[ROWS], ROWS as f64); // first element of column 1
}

#[test]
fn clone_shares_the_block_rather_than_copying_it() {
    let df = block_frame();
    let cloned = df.clone();

    let a = df.to_numpy_block_view().expect("original is block-backed");
    let b = cloned.to_numpy_block_view().expect("clone stays block-backed");

    // The whole point of the primitive: the clone must SHARE the payload. Pointer
    // identity of the backing allocation is the actual claim -- equal contents would
    // also hold for a deep copy, so comparing values would not test anything.
    assert!(
        std::sync::Arc::ptr_eq(&a.block, &b.block),
        "clone must share the Arc<[f64]> block, not duplicate it"
    );
}

#[test]
fn transforming_one_frame_does_not_disturb_a_sharing_sibling() {
    // The independence proof, inverted from franken_networkx's staleness proof.
    let df = block_frame();
    let sibling = df.clone();
    let before: Vec<f64> = sibling
        .to_numpy_block_view()
        .expect("sibling is block-backed")
        .block
        .to_vec();

    // Every DataFrame "mutation" in this crate is value-semantics: it returns a NEW
    // frame. Exercise the ones that could plausibly reach the block.
    let added = df
        .with_column("added", Column::from_f64_values(vec![9.0; ROWS]))
        .expect("with_column");
    let dropped = df.drop_column("c0").expect("drop_column");
    let renamed = df.rename(&[("c1", "renamed")]).expect("rename");

    // The sibling's block must be byte-for-byte what it was.
    let after: Vec<f64> = sibling
        .to_numpy_block_view()
        .expect("sibling still block-backed")
        .block
        .to_vec();
    assert_eq!(
        before, after,
        "a transformation of one frame was observable through a frame sharing its block"
    );

    // And the originals are untouched -- value semantics, not in-place edits.
    assert_eq!(df.column_names().len(), COLS);
    assert_eq!(added.column_names().len(), COLS + 1);
    assert_eq!(dropped.column_names().len(), COLS - 1);
    assert_eq!(renamed.column_names().len(), COLS);
}

#[test]
fn per_column_reads_agree_with_the_block_layout() {
    // Guards the borrow: each column is a span of the shared block, so a read through
    // the normal columnar API must agree with the raw column-major view.
    let df = block_frame();
    let view = df.to_numpy_block_view().expect("block-backed");
    for c in 0..COLS {
        let name = format!("c{c}");
        let col = df.column(&name).expect("column present");
        let values = col.values();
        for (r, value) in values.iter().enumerate().take(ROWS) {
            let expected = view.block[c * ROWS + r];
            assert_eq!(
                *value,
                fp_types::Scalar::Float64(expected),
                "column {name} row {r} disagrees with block[{c}*{ROWS}+{r}]"
            );
        }
    }
}
