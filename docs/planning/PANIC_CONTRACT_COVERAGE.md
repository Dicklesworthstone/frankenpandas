# Panic-Contract Coverage & Public-API Panic Inventory

Per `br-frankenpandas-z9a8` (follow-up to `br-frankenpandas-7cfm`) and
`br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.13` (Public-API Panic Audit).

**Policy (README Bedrock Invariant #4).** Every public function that can fail on
user or adversarial input returns a typed `Result` rather than panicking. Any
remaining `.unwrap()`, `.expect(...)`, `unreachable!()`, or `panic!()` in
non-test library code must either:
1. Be **converted to a typed `Result`** (with a negative/regression unit test
   feeding the formerly panicking input),
2. Be **proven unreachable** by an immediately preceding or structural invariant
   (documented with a clear `.expect("...")` / `unreachable!("...")` invariant
   citation), or
3. Document its explicit `# Panics` contract in rustdoc (for intentionally
   panicking assertion macros or infallible capacity helpers).

---

## 1. Non-Test Panic Surface Census (`br-frankenpandas-4qg5w.13`)

Machine-verified scan across all non-test library sources (`crates/*/src/**/*.rs`,
excluding `#[cfg(test)]` modules and `debug_assert*!` macros):

| Crate | `.unwrap()` | `.expect()` | `unreachable!()` | `panic!()` | `todo!()` / `unimplemented!()` | Status |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| `fp-types` | 0 | 0 | 0 | 0 | 0 | Zero non-test panic macros |
| `fp-expr` | 0 | 0 | 0 | 0 | 0 | Zero non-test panic macros |
| `fp-runtime` | 0 | 0 | 0 | 0 | 0 | Zero non-test panic macros |
| `fp-dot-kernel` | 0 | 0 | 0 | 0 | 0 | Zero non-test panic macros |
| `frankenpandas` | 0 | 0 | 0 | 0 | 0 | Zero non-test panic macros |
| `fp-io` | 0 | 19 | 2 | 0 | 0 | Zero `.unwrap()` / `panic!()`; all 21 sites invariant-guarded |
| `fp-groupby` | 0 | 29 | 1 | 0 | 0 | Zero `.unwrap()` / `panic!()`; all 30 sites invariant-guarded |
| `fp-index` | 0 | 43 | 2 | 0 | 0 | Zero `.unwrap()` / `panic!()`; 39 invariant-guarded + 4 capacity-overflow guards |
| `fp-columnar` | 0 | 37 | 19 | 0 | 0 | Zero `.unwrap()` / `panic!()`; all 56 sites invariant-guarded |
| `fp-join` | 0 | 82 | 1 | 0 | 0 | Zero `.unwrap()` / `panic!()`; all 83 sites invariant-guarded |
| `fp-frame` | 52 | 372 | 42 | 6 | 0 | All 6 `panic!()` are `assert_*_eq!` test-assertion helpers; all other sites invariant-guarded |

---

## 2. Converted User-Data-Reachable Panic Sites (`br-frankenpandas-4qg5w.13`)

Every site below was reachable from public APIs via adversarial or edge user
inputs and has been converted to return a typed `Result` (or safe fallback) with
a dedicated regression test:

| Location (`file:line`) | Public Reachability Path | Former Failure Mode | Disposition | Regression Test |
| --- | --- | --- | --- | --- |
| `crates/fp-types/src/lib.rs:10145` | `Interval::parse(s)` | `.unwrap()` on `s.chars().next()` / `last()` and byte slice `&s[1..s.len() - 1]` panicked on multi-byte UTF-8 inputs of byte length >= 5 whose first/last codepoint exceeded 1 byte (e.g. `"€€"` = 6 bytes, byte 1 inside `'€'`) | Converted to `s.strip_prefix(['[', '('])` and `after_left.strip_suffix([']', ')'])`, returning `Err(TypeError::ValueNotParseable)` | `fp-types::pandas_decimal_py3c0::interval_parse_rejects_multibyte_and_malformed_inputs_without_panic_4qg5w_13` |
| `crates/fp-expr/src/lib.rs:1527` | `evaluate_scalar_operand_named(expr, ctx, name)` via `eval` / `eval_with_locals` | `unreachable!("guarded by is_scalar_operand")` if called with a non-scalar `Expr` variant | Converted to `Err(ExprError::UnanchoredLiteral)` | `fp-expr::eval_value_2blaf::evaluate_scalar_operand_named_rejects_non_scalar_without_panic_4qg5w_13` |
| `crates/fp-frame/src/plot_render.rs:188` | `Series::plot_barh()?.to_svg()` / `DataFrame::plot_barh()?.to_svg()` -> `svg_horizontal_axes` | `&label[..7]` when `label.len() > 8` panicked with a UTF-8 char-boundary panic whenever a category label contained multi-byte UTF-8 characters spanning byte index 7 (e.g. `"αβγδεζη"` = 14 bytes, 7 chars) | Converted to `label.chars().count() > 8` and `label.chars().take(7).collect::<String>()` | `fp-frame::plot_render::tests::barh_plot_multibyte_utf8_labels_do_not_panic_4qg5w_13` |
| `crates/fp-frame/src/plot_render.rs:870` | `PlotSpec::to_svg()` -> `render_plot_with_bw_method` | `PlotKind::Histogram \| PlotKind::Box => unreachable!()` | Replaced `unreachable!()` with direct delegation to `histogram_body` and `boxplot_body` | `fp-frame::plot_render::tests::barh_plot_multibyte_utf8_labels_do_not_panic_4qg5w_13` |
| `crates/fp-frame/src/lib.rs:70223` | `Series::dt().tz_localize(...)` / `Series::dt().floor(...)` / `ceil` / `round` -> `fixed_offset_suffix` & `strip_tz_suffix` | `&trimmed[trimmed.len() - 6..]` and `&without_zone[..without_zone.len() - 6]` panicked with a UTF-8 char-boundary panic when a string had multi-byte UTF-8 characters spanning `len - 6` (e.g. `"2024-01-01 €€a"`) | Converted to `trimmed.get(trimmed.len() - 6..)?` and `strip_suffix(suffix)` | `fp-frame::test_top_level_and_reshaping::dt_tz_localize_and_floor_multibyte_utf8_do_not_panic_4qg5w_13` |
| `crates/fp-frame/src/lib.rs` (`pct_change`, `shift`, `diff`, `repeat`, `repeat_by`) | `Series::pct_change(i64::MIN)` / `Series::repeat(usize::MAX)` / `Series::repeat_by` | Arithmetic overflow on `-periods` at `i64::MIN` and capacity overflow on `len * repeats` | Checked arithmetic (`saturating_neg`, `checked_mul`, `try_reserve`) returning typed error or clamped NaN fill | `fp-frame::tests::extreme_periods_and_repeats_are_answers_not_panics_4qg5w_13` |
| `crates/fp-frame/src/lib.rs` (`Rolling::quantile`, `Expanding::quantile`, `Series::nunique`) | `rolling(w).quantile(q)` with `q` outside `[0, 1]` or `NaN`; `Series::nunique(dropna=False)` on numeric object column with missing value | Unchecked quantile clamping / `unreachable!()` on missing object cell in `nunique` | Converted to typed `FrameError` (`quantile` out of `[0, 1]`) and explicit missing-cell arm | `test_extreme_arguments_raise_as_pandas_instead_of_panicking` |
| `crates/fp-columnar/src/lib.rs` (`floordiv` / `mod` with `Bool`) | `Column::binary_numeric_op` (`Int64 // Bool`, `Int64 % Bool`) | Reached `unreachable!()` on `Bool` operand in integer division/modulo | Converted to promote `Bool` to `Int64` / return typed division-by-zero behavior (`kyvo0.3`) | `fp-columnar` integer division/modulo bool tests |
| `crates/fp-python/src/lib.rs` (Sweeps 1–5: `7c7649b86`, `afd01d560`, `3c1f28656`, `cf1db2eb2`, `7868ab9a9`) | Python public API surface across `Series`, `DataFrame`, `Index`, `Timestamp`, `Timedelta`, `.str`, `.dt`, `.cat`, `GroupBy`, `Rolling`, `Resampler`, `interval_range`, `cut`, `value_counts` | Overflow panics (`Timestamp.replace(year=1)`, `Timedelta(days=999999999)`), allocation aborts (`str * sys.maxsize`, `str.pad(huge)`, `value_counts(bins=10**18)`), infinite loop (`interval_range(freq=0)`), and GIL worker deadlock (`groupby(dates).rank()`) | Converted to `OutOfBoundsDatetime`, `OutOfBoundsTimedelta`, `MemoryError`, `ValueError`, `OverflowError`, and inline execution when host Python objects are live | `test_extreme_arguments_raise_as_pandas_instead_of_panicking`, `test_interval_range_breaks_match_pandas`, `test_parallel_kernels_over_python_objects_do_not_deadlock` |

---

## 3. Disposition Ledger of Remaining Non-Test Sites by Crate

### 3.1 `fp-types`, `fp-expr`, `fp-runtime`, `fp-dot-kernel`, `frankenpandas`
- **0 non-test `.unwrap()`, `.expect()`, `unreachable!()`, `panic!()`, or `todo!()` sites.**

### 3.2 `fp-io` (0 `.unwrap()`, 19 `.expect()`, 2 `unreachable!()`, 0 `panic!()`)
All 21 sites are guarded by immediate structural invariants:
- `crates/fp-io/src/lib.rs:1011, 1032, 1035` (`expect`): `usize::try_from(row_count)` and `rows.checked_mul(cols)` in fast CSV parser after `row_count >= 0` and input byte length `<= isize::MAX` are validated.
- `crates/fp-io/src/lib.rs:1039` (`unreachable!`): `unreachable!("all typed CSV columns were checked as Float64")` inside a block gated by `col_dtypes.iter().all(|d| *d == DType::Float64)`.
- `crates/fp-io/src/lib.rs:2006, 2183` (`expect`): `std::str::from_utf8` on byte slices carved at ASCII delimiter boundaries from an already-validated `&str` input.
- `crates/fp-io/src/lib.rs:2301, 2305` (`expect`): `implicit_index_labels.as_mut()` and `fields.first()` gated on `implicit_index` mode and `fields.len() == headers.len() + 1`.
- `crates/fp-io/src/lib.rs:3379, 8720, 9002, 9033, 9867` (`expect`): `thread::ScopedJoinHandle::join()` propagating worker thread completion where worker closures perform pure data formatting/parsing without panicking.
- `crates/fp-io/src/lib.rs:4678, 7128` (`expect`): `Option::take()` / `.expect("checked immediately above")` immediately after `is_some()` guard.
- `crates/fp-io/src/lib.rs:8742, 8761` (`expect`) & `8765` (`unreachable!`): JSON columnar numeric parser after `width` and `is_float` pre-validation across all chunks.
- `crates/fp-io/src/lib.rs:8921, 9194` (`expect`): `cols[ci].last_mut().expect("seen this record => non-empty")` immediately after pushing a default slot for the current record.
- `crates/fp-io/src/lib.rs:11646` (`expect`): Arrow `Float64Array` downcast on a column whose Arrow schema `DataType::Float64` was just matched.

### 3.3 `fp-groupby` (0 `.unwrap()`, 29 `.expect()`, 1 `unreachable!()`, 0 `panic!()`)
- `crates/fp-groupby/src/lib.rs:511, 588, 609, 797, 807, 883, 893, 995, 1005, 1927, 1937, 2004, 2014, 2097, 2107, 2186, 2196, 2274, 2284, 2368, 2378, 2456, 2466, 2531, 2541` (`expect`): `.expect("ordering references only inserted keys")` when indexing the group accumulator map by keys recorded in `ordering` during the same pass.
- `crates/fp-groupby/src/lib.rs:1647` (`unreachable!`): `_ => unreachable!("dense path gated to the supported aggregations")` inside `groupby_dense_int64`, which is entered only after matching the supported `AggFunc` set.
- `crates/fp-groupby/src/lib.rs:3348, 3369, 3427, 3449` (`expect`): Arena/slot lookups for group IDs bounded by `0..num_groups`.

### 3.4 `fp-index` (0 `.unwrap()`, 43 `.expect()`, 2 `unreachable!()`, 0 `panic!()`)
- `crates/fp-index/src/lib.rs:994, 1008, 1036, 1059, 1087, 1114, 1141, 1168` (`expect`): `RwLock::read()` / `write()` on internal lookup caches (never held across unwind boundaries).
- `crates/fp-index/src/lib.rs:1232, 1236, 1271, 1285, 1313, 1317, 1320, 1384, 1971, 1980, 1996, 2005, 2038, 2202` (`expect`): Checked `i64` range arithmetic on `Index` affine/unit ranges whose bounds were validated at construction (`Index::new_known_unique_int64_unit_range` / `AffineRange`).
- `crates/fp-index/src/lib.rs:1838, 1858, 1903` (`expect`): Contiguous UTF-8 buffer and lazy label invariants enforced at `IndexLabels` construction.
- `crates/fp-index/src/lib.rs:2952, 2957, 2994, 3001` (`expect`): `repeat_output_capacity`, `combined_output_capacity`, `insert_output_capacity`, `aggregate_output_capacity` — intentional overflow guards on infallible `Index::repeat` / `append` / `insert` methods (`uza04182`, `uza04183`, `uza04185`); the Python binding pre-validates `len.checked_mul(repeats)` to raise `ValueError` before calling these helpers.
- `crates/fp-index/src/lib.rs:25569, 25572` (`unreachable!`): `unreachable!("two_utf8_level_slices pre-validates all labels")` after verifying both MultiIndex levels are `Utf8`.
- Remaining 14 `.expect()` sites: Internal slice/bucket lookups bounded by `0..self.len()` or `column_names` membership.

### 3.5 `fp-columnar` (0 `.unwrap()`, 37 `.expect()`, 19 `unreachable!()`, 0 `panic!()`)
- `crates/fp-columnar/src/lib.rs:3330, 3341` (`unreachable!`): `ArithmeticOp::Div | ArithmeticOp::Pow => unreachable!("handled by early return above")` after early-returning for `Div` and `Pow`.
- `crates/fp-columnar/src/lib.rs:9242, 22162, 22369, 22373, 29820, 29827, 30292, 30340, 30478, 30535, 30572, 30602, 30633, 30667, 30734, 30766, 30866` (`unreachable!`): Type-specialized kernel arms entered only after pre-checking `dtype` and all-valid/homogeneous scalar tags.
- 37 `.expect()` sites: Validated UTF-8 contiguous offset buffers (`expect("contiguous utf8 buffer is valid by construction")`), scoped worker thread joins (`expect("column worker panicked")`), and pre-validated dense-cycle key spans.

### 3.6 `fp-join` (0 `.unwrap()`, 82 `.expect()`, 1 `unreachable!()`, 0 `panic!()`)
- `crates/fp-join/src/lib.rs:290, 742, 754, 839, 850, 10307, 10326, 10348` (`expect`): `left_map` / `right_map` presence gated by `matches!(join_type, JoinType::Outer | JoinType::Right)` when constructing the maps immediately above.
- `crates/fp-join/src/lib.rs:1378, 10401, 10405` (`expect`): `rp.expect("an unmatched merge row has a right position")` — in an outer/right join row where `lp` is `None`, `rp` is `Some` by construction.
- `crates/fp-join/src/lib.rs:5416` (`unreachable!`): `(None, None) => unreachable!("empty buckets are skipped above")` after `if left_slice.is_empty() && right_slice.is_empty() { continue; }`.
- Remaining 71 `.expect()` sites: `left.columns().get(name).expect("left column listed in column_names must exist")` (iterating `left.column_names()`), scoped worker thread joins, and pre-sized output lane buffers.

### 3.7 `fp-frame` (52 `.unwrap()`, 372 `.expect()`, 42 `unreachable!()`, 6 `panic!()`)
- `crates/fp-frame/src/lib.rs:1174, 1179, 1189, 1194, 1204, 1209` (`panic!`): Inside `assert_frame_eq!`, `assert_series_eq!`, and `assert_index_eq!` macros, which exist specifically to panic on test assertion failure (documented `# Panics` contract).
- 42 `unreachable!()` sites: All gated by preceding dtype/method/variant checks (`mapping shape checked above`, `anchored_asfreq_anchors called with a fixed unit`, `typed unit-range gate admits only numeric peers`, `gated on all-Int64 needles above`, `rank method validated`, `lazy transpose plan only admits certified dense dtypes`, `GroupByRolling agg output is nullable-f64`).
- 52 `.unwrap()` & 372 `.expect()` sites: Audited in `crates/fp-frame/UBS_TRIAGE.md` and `artifacts/audits/fp_frame_ubs_inventory_2026-06-17.md`; all are guarded by immediate `is_some()` / `!is_empty()` / `columns.contains_key` checks, `OnceLock` / `Mutex` caches, or scoped worker thread joins.
