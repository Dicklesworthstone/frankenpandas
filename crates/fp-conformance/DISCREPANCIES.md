# Known Conformance Divergences

> Every intentional divergence from pandas behavior is documented here.
> Format: DISC-NNN, status (ACCEPTED/INVESTIGATING/WILL-FIX), affected tests.

## Active Divergences

### DISC-001: Integer division by zero promotes to Float64 with NaN/inf
- **Reference:** pandas `int64 // int64` with zero divisor returns `float64` with `inf`
- **Our impl:** Same behavior - promotes to Float64, returns `inf` for floor division, `nan` for mod
- **Impact:** Dtype promotion matches, values match
- **Resolution:** ACCEPTED - exact pandas parity achieved
- **Tests affected:** `int64_mod_floordiv_with_zero_promotes_to_float`
- **Review date:** 2026-04-15

### DISC-002: Unicode width tables version
- **Reference:** pandas uses system's ICU or Python's unicodedata (varies by install)
- **Our impl:** Uses `unicode-width` crate (Unicode 15.1 tables)
- **Impact:** Some emoji/CJK width calculations may differ by 1 column
- **Resolution:** ACCEPTED - newer Unicode tables are more correct
- **Tests affected:** None currently - string display width not yet tested
- **Review date:** 2026-04-15

### DISC-003: Error message text differs
- **Reference:** pandas error messages vary by version and locale
- **Our impl:** Custom error messages with consistent format
- **Impact:** Error semantics match, exact text differs
- **Resolution:** ACCEPTED - tests check error category, not message text
- **Tests affected:** All error-expecting tests use `expected_error_contains`
- **Review date:** 2026-04-15

### DISC-004: CSV NA value handling default differs from pandas 1.x
- **Reference:** pandas 2.x treats "None" as NA by default; pandas 1.x did not
- **Our impl:** Follows pandas 2.x behavior with `keep_default_na=true`
- **Impact:** Users migrating from pandas 1.x may see different behavior
- **Resolution:** ACCEPTED - aligning with current pandas 2.x
- **Tests affected:** `csv_none_is_default_na`
- **Review date:** 2026-04-15

### DISC-006: Row MultiIndex is scaffolded, not full pandas parity
- **Reference:** pandas `MultiIndex` supports arbitrary-level hierarchical row labels with full slicing, `xs`, `droplevel`, `swaplevel`, `reindex`, `sort_index`, etc.
- **Our impl:** Row-MultiIndex first slice ships struct + constructor + level access. Full slicing / xs / droplevel / swaplevel coverage lands in subsequent slices (umbrella tracked by br-frankenpandas-1zzp).
- **Impact:** DataFrames built with a row MultiIndex may reject operations that pandas accepts, or return partial results. Error messages identify which operation is pending.
- **Resolution:** INVESTIGATING - slices land under br-1zzp child beads until coverage parity is reached.
- **Tests affected:** `live_oracle_dataframe_row_multiindex_*` suite (scoped to shipped operations).
- **Review date:** 2026-04-23

### DISC-007: SQL IO is SQLite-only; pandas supports multiple backends
- **Reference:** pandas `read_sql` / `to_sql` accept any SQLAlchemy-compatible backend (SQLite, PostgreSQL, MySQL, Oracle, MSSQL, etc.).
- **Our impl:** fp-io's `read_sql` / `write_sql` only accept a `rusqlite::Connection`. PostgreSQL / MySQL / Oracle connectors not shipped.
- **Impact:** Users whose pipelines depend on non-SQLite backends cannot drop-in replace pandas IO calls.
- **Resolution:** INVESTIGATING - tracked by br-frankenpandas-fd90 (SQL backend epic, 7 slices). SQLite remains the supported scope until slices 2+ land.
- **Tests affected:** `live_oracle_sql_*` suite (SQLite only).
- **Review date:** 2026-04-23

### DISC-008: No Python bindings shipped; pandas' Python-level drop-in positioning differs
- **Reference:** pandas IS a Python library. Users `import pandas`.
- **Our impl:** frankenpandas is a Rust library. Users `use frankenpandas::*` from Rust code. Python bindings (e.g. via PyO3) are not shipped.
- **Impact:** README's "drop-in pandas replacement" positioning applies at the API-shape level, not at the import-statement level. A Python pandas user cannot adopt frankenpandas without first porting to Rust.
- **Resolution:** ACCEPTED - README was updated to qualify the claim (br-frankenpandas-diic closed by wording rather than bindings). PyO3 bindings remain a future-epic candidate, not in scope for 0.1.0.
- **Tests affected:** N/A - positioning / documentation divergence, not behavioral.
- **Review date:** 2026-04-23

### DISC-009: Sparse dtype descriptor exists before compressed sparse storage
- **Reference:** pandas `SparseDtype` pairs an underlying value dtype with a fill value and stores only non-fill positions in `SparseArray`.
- **Our impl:** `fp-types::SparseDType` records the dtype/fill-value contract and `DType::Sparse` marks the logical dtype. fp-columnar still stores columns densely and IO falls back to textual sparse markers until a compressed sparse column representation lands.
- **Impact:** Code can now describe sparse dtype intent, but memory usage and `Series.sparse` accessor parity still differ from pandas.
- **Resolution:** WILL-FIX - remaining storage/accessor work tracked by br-frankenpandas-0xcm follow-up slices.
- **Tests affected:** Sparse storage/accessor conformance tests not yet enabled.
- **Review date:** 2026-04-24

### DISC-010: Rust GroupBy.apply uses explicit output-shape APIs
- **Reference:** pandas `DataFrameGroupBy.apply` dynamically dispatches scalar, Series, and DataFrame return values from one Python callable.
- **Our impl:** Rust's static return types expose the same shape families as explicit methods: `apply_scalar`, `apply_series`, `apply_series_stacked`, and DataFrame-returning `apply`. DataFrame-returning apply retains group-key row MultiIndex metadata; stacked Series output is represented as a one-column DataFrame until Series row MultiIndex metadata lands.
- **Impact:** Shape semantics are available, but Rust callers choose the expected output family at compile time instead of receiving a dynamic Python object.
- **Resolution:** INVESTIGATING - a future Python binding layer can restore one-call dynamic dispatch over these Rust shape-specific methods.
- **Tests affected:** `dataframe_groupby_apply`, `dataframe_groupby_apply_scalar_returns_series_indexed_by_keys`, `dataframe_groupby_apply_series_unions_sparse_result_columns`, `dataframe_groupby_apply_series_stacked_preserves_variable_labels`.
- **Review date:** 2026-04-25

### DISC-012: Mixed naive / tz-aware CSV parse_dates normalizes per value
- **Reference:** Pandas handles a CSV column with mixed naive + tz-aware datetime strings by parsing each row independently (the naive rows produce `Timestamp` without tz; the aware rows produce `Timestamp` with tz). When converted to strings, both forms are reformatted into pandas' canonical `YYYY-MM-DD HH:MM:SS[±HH:MM]` shape.
- **Our impl:** fp-io now parses `read_csv(parse_dates=[...])` mixed naive + tz-aware columns per value by calling `to_datetime_values_with_options` with `infer_mixed_timezone=false` and `mixed_tz_as_object=true`. The column remains object-like (`Utf8`) because pandas cannot unify mixed tz-naive/tz-aware values into one `datetime64[ns]` dtype, but each value is normalized to the pandas object-string form.
- **Impact:** Conformance packet `FP-P2D-429` (`csv_read_frame_parse_dates_mixed_timezone_strict`) now matches the fixture: the aware row is normalized from `2024-01-15T10:30:00Z` to `2024-01-15 10:30:00+00:00`.
- **Resolution:** RESOLVED — covered by fp-io test `csv_parse_dates_mixed_naive_and_aware_strings_normalizes_per_value`; the stale accepted-divergence note was superseded by the per-value parse path used by `parse_csv_datetime_values`.
- **Tests affected:** none expected; historical coverage remains `packet_filter_runs_csv_read_frame_parse_dates_mixed_timezone_packet`.
- **Review date:** 2026-06-17

### DISC-015: memory_usage exact bytes differ from pandas (structural divergence)
- **Reference:** pandas `DataFrame.memory_usage()` reports exact bytes consumed by numpy-backed columns. For the test frame in `FP-P2D-364`, pandas returns 234 bytes (index + column overhead + numpy array backing).
- **Our impl:** FrankenPandas uses `Vec<Scalar>` storage which has structurally different memory characteristics. The same frame reports 32 bytes — a 7x difference reflecting heap-allocated scalars vs numpy's contiguous buffer layout.
- **Impact:** Conformance packet `FP-P2D-364` (`dataframe_memory_usage_with_nulls_hardened`) fails with `actual=32, expected=234`. This is NOT a bug but a fundamental structural difference.
- **Resolution:** ACCEPTED — exact-byte parity is impossible without adopting numpy's physical layout. Documented in README Memory Model section. Relative/shape assertions remain valid (larger frames use more memory monotonically). Excluded from parity-score numerator via fixture waiver.
- **Tests affected:** `FP-P2D-364`, any exact memory_usage comparison tests.
- **Review date:** 2026-05-25
- **Waiver:** Signed by user request per br-frankenpandas-rg8ys.5.2.

### DISC-011: Int64 columns receiving null values promote to Float64 (no nullable Int64 extension dtype)
- **Reference:** Pandas (since v0.24) has a nullable `Int64` extension dtype (capital I) that preserves the integer encoding via a separate validity mask. When a non-nullable `int64` column receives a null (e.g. via index alignment introducing rows with no source data, or via `concat(axis=1)` aligning over a non-matching index), pandas can either preserve `Int64` (extension) or promote to `float64` depending on dtype. The conformance oracle uses extension `Int64` where the column was originally `int64`.
- **Our impl:** No nullable extension Int64 dtype yet. Int64 columns that gain null values are promoted to `Float64` with `NaN`. Downstream IO (JSON, CSV) then serializes the integer values with a trailing `.0` (`1.0` rather than `1`).
- **Impact:** Several conformance packets exhibit `actual=Float64(1.0), expected=Int64(1)` mismatches:
  - `FP-P2D-028` (dataframe_concat_axis1): 5 of 10 cases fail because alignment over a wider index introduces nulls into formerly-Int64 columns.
  - `FP-P2D-433` (dataframe_to_json_records): JSON output writes `"a":1.0` instead of `"a":1` for integer columns that were promoted via null introduction.
  - Plus other downstream packets where alignment + nulls hit Int64 columns.
- **⚠️ CORRECTION 2026-08-06 (br-frankenpandas-fixture-divergence-triage-9s0c4): the "Our impl" line above is NOT true of every path, and the difference decides whether ~97 fixtures get regenerated.** On the **merge and concat** paths FrankenPandas does NOT promote — it keeps `Int64` and carries the missing value in the validity mask, i.e. exactly the `int64 + null` the fixtures pin. Measured: `packet_filter_runs_dataframe_merge_sort_packet` (FP-P2D-037, a `how="right"` merge with an unmatched row) PASSES against a fixture pinning `left_v -> int64 [10, null, 30]`, and the whole `fp-conformance --lib` suite is green against fixtures of this shape. Live pandas 2.2.3 on the identical input returns `left_v -> float64 [10.0, NaN, 30.0]`; same story for `concat(axis=0, join="outer")`, where pandas gives `float64 [NaN, NaN, 300.0]` and the fixture pins `int64 [null, null, 300]`. So the divergence is real and is this DISC, but the mechanism is "FP represents Int64-with-nulls where pandas cannot" rather than "FP promotes like pandas does". Whoever implements the nullable-Int64 epic should re-derive which paths promote and which do not before trusting the original sentence.
- **Fixture-corpus impact, and why these must NOT be regenerated:** the live-pandas differ attributes **97 of its 181 divergent rows** to this one cause — 46 spelled `int64` vs pandas `float64`, 51 spelled `null` vs pandas `NaN`; they are the same phenomenon seen once in the dtype and once in the missing-value marker. Those fixtures pin the extension-`Int64` behavior this DISC is WILL-FIXing toward. Regenerating them to `float64 + NaN` would make the divergence vanish from the corpus and delete the pinned evidence of a tracked architectural gap — the golden-regeneration reflex at scale. They stay as they are, now with a named cause instead of an unexplained bucket.
- **Resolution:** WILL-FIX - implementing nullable extension Int64 is a significant architectural change touching storage (fp-columnar), arithmetic kernels (fp-frame), and serialization (fp-io). Tracked under a future epic, not in scope for the fd90 SQL backend work. Per br-frankenpandas-mywg (fd90.76).
- **Tests affected:** `packet_filter_runs_dataframe_concat_axis1_packet`, `packet_filter_runs_dataframe_to_json_records_packet`, `fuzz_json_io_bytes_accepts_records_seed_fixture` (the records seed has `[{"temp":72},{"temp":null}]` — read promotes to Float64, write emits `72.0` instead of `72`, reparse + diff detects the drift), plus other downstream packets that hit the same root cause.
- **Review date:** 2026-08-06 (corrected; was 2026-04-26)

### DISC-018: Timedelta/Timestamp arithmetic overflow surfaces as NaT (pandas raises)
- **Reference:** pandas 2.2.3 is **not uniform** here, which is the load-bearing fact. Probed live against the installed oracle:
  - **RAISES** — `Series + Timedelta`, `Series - Series`, `Series.diff` → `OverflowError: Overflow in int64 addition`; `Series.sum()`, `Series.std()` → `ValueError: overflow in timedelta operation`; `Timedelta.max + Timedelta('1ns')` → `OverflowError`; `Timestamp.max + Timedelta('1ns')` → `OutOfBoundsDatetime`; `Series([tmin,tmax]).max() - .min()` (the ptp expression) → `OverflowError`.
  - **WRAPS SILENTLY** — `Series.cumsum()` and `Series * 2` wrap modulo 2^64 with no exception (raw numpy int64), e.g. `Timedelta.max * 2` → `-1 days +23:59:59.999999998`. `Period` arithmetic likewise wraps with no check at all.
  - **Returns NaT** — `Series([tmax, tmax]).mean()` and `.median()` and `.quantile(0.5)` (pandas' own internal float cast overflows); `Series * 2.0` with a **float** multiplier (contrast the int multiplier one line above, which wraps); `Series / 0`, `Series // 0`, `Series / 0.0`, and the overflowing `Series / 0.5` — the whole vectorized division family; `Series([0, tmax]).sum()`, where the exact answer *is* representable but pandas' f64 detour rounds `2^63 - 1` up to `2^63` and the cast back to int64 lands on the NaT sentinel.
  - **AGREES WITH US BY SENTINEL COLLISION** — `Timedelta.min - Timedelta('1ns')` → NaT, scalar *and* vectorized, and likewise `Timestamp.min - 1ns`. This is not a policy: `Timedelta.min.value` is `i64::MIN + 1`, so one step below it lands exactly on `i64::MIN`, which pandas also reserves as its NaT sentinel. **Two** steps below (`min - 2ns`) raises `OverflowError`. Any test that pins only the 1 ns case proves nothing about this divergence — it passes for a correct and an incorrect implementation alike.
  - **NOT AN OVERFLOW AT ALL** — `-Timedelta.min`, `abs(Timedelta.min)`, and their vectorized forms return `Timedelta.max`. pandas' range is symmetric about zero precisely because `i64::MIN` is spent on NaT. FrankenPandas matches, and `neg`/`abs` carried a doc comment claiming the opposite until br-frankenpandas-fyr1z corrected it.
- **Our impl:** FrankenPandas returns **NaT** wherever the exact result is unrepresentable, uniformly, and keeps the helpers infallible (`#[must_use]`, no `Result`). This covers the scalar boundary (`Timedelta::{add,sub,mul_scalar}`, `Timestamp::{add_timedelta,sub_timedelta}`, `Timedelta::from_unit`) and the vectorized reduction surface (`nansum`, `nanptp`, `nancumsum`). The representable range is `[i64::MIN + 1, i64::MAX]`, since `i64::MIN` is the NaT sentinel.
- **Impact:** A caller who overflows gets a missing value where pandas would raise, and NaT is indistinguishable from a genuinely missing input. This is a deliberate trade, not an oversight: the alternative previously in the tree was **saturation**, which fabricated a finite ~106751-day Timedelta and presented it as real data — fail-open, and strictly worse than either pandas behavior. Fail-closed-as-missing beats fail-open-as-plausible-data. Where pandas *wraps*, we deliberately do not chase the wrap (wrapping is not better behavior than NaT, and matching it would need its own justification).
- **Resolution:** ACCEPTED for the current API shape. The observability decision (`br-frankenpandas-fyr1z`) is now **DECIDED: a per-op strict/hardened mode split**, with one correction that the decision turns on — **a blanket "STRICT raises" is NOT pandas parity and would make things worse.** pandas refuses only on part of this surface; on the rest it returns NaT itself (division family, float multipliers, mean/median/quantile, the 1 ns sentinel step) or silently wraps (int multiplier, `cumsum`). Making STRICT raise uniformly would *introduce* divergence in every row of the "Returns NaT" and "agrees by sentinel collision" groups above, where FrankenPandas is already bit-for-bit correct. STRICT must therefore be specified **per operation** against the measured table above, not as a global policy; HARDENED keeps today's uniform NaT plus an audit-log entry. Two consequences worth stating plainly: STRICT parity for `Series * 2` and `cumsum` means **reproducing a silent two's-complement wrap**, which is fail-open behavior the repo otherwise forbids and which needs its own explicit sign-off; and the divergence surface that actually needs new code is much smaller than "every overflow". Escalating the fp-types helpers to `Result` (option (b)) is rejected as the primary mechanism — it would force ~12 elementwise call sites to re-litigate raise-vs-propagate while fixing none of the vectorized rows, which is where users meet this. Precedent: `Timedelta::div_scalar` returns NaT on divide-by-zero, which the scalar pandas API raises on (`ZeroDivisionError`) but the vectorized API agrees with.
- **Tests affected:** `nansum_timedelta_overflow_is_nat_not_fabricated_max_opz27`, `nansum_timedelta_negative_overflow_is_nat_opz27`, `nanptp_timedelta_overflow_is_nat_opz27`, `nancumsum_timedelta_overflow_recovers_exact_value_opz27`, `nancumsum_timedelta_exact_path_unchanged_opz27`, `arithmetic_overflow_never_fabricates_a_finite_value_lgyy8` (the divergence half), `overflow_divergence_surface_is_only_where_pandas_refuses_fyr1z` (the agreement half plus the 1-step/2-step sentinel pair), `neg_and_abs_at_the_representable_floor_are_not_overflow_fyr1z` (all in `fp-types`). Beads: `br-frankenpandas-lgyy8`, `br-frankenpandas-8v92m`, `br-frankenpandas-opz27`; decision recorded by `br-frankenpandas-fyr1z`, implementation decomposed into its children.
- **Review date:** 2026-08-06

### DISC-019: Datetime64 mean/median/quantile are exact in FrankenPandas; pandas snaps to its f64 grid
- **Reference:** pandas 2.2.3 does not compute datetime64 `mean`/`median`/`quantile` on the int64 nanosecond backing — it detours through float64. At realistic timestamps this loses precision outright, because the f64 ULP at 2020-01-01 (1.578e18 ns) is **256 ns**. Probed live:
  - `Series([B+1, B+1]).mean()` → `B+0`. A **constant** series, where no averaging is needed and the answer is unambiguous, still comes back wrong by 1 ns.
  - `Series([B, B+129, B+258]).median()` → `B+256`. An odd-count median is a pure **selection** of an existing element, yet the returned Timestamp **is not one of the inputs**.
  - `quantile(0.5)` on the same input → `B+256`.
  - Rounding on that 256 ns grid is ties-to-even (`B+128` → `B+0`, `B+129` → `B+256`).
  - Root cause reproducible directly: `s.values.view('i8').mean()` returns `1.5778368e+18`, dropping the low bits, while `i8.sum() // 2` is exact. By contrast `min`/`max` are exact, since they are integer operations.
- **Our impl:** FrankenPandas computes `nanmean`/`nanmedian` for Datetime64 in **i128 nanoseconds, exactly**, and narrows with Rust's toward-zero integer division. Where pandas is exact (small magnitudes), FP matches it bit-for-bit **including the rounding rule**, which was probed and is truncation toward zero — `[0,1]→0`, `[0,3]→1`, `[-1,0]→0`, `[-3,0]→-1`, `[1,2]→1`. Both `floor` (would give `-1` for `[-1,0]`) and ties-to-even (would give `2` for `[1,2]`) are refuted. Where pandas snaps to its f64 grid, FP stays exact. `nanquantile` deliberately keeps the f64 hop its Timedelta64 sibling already used, because quantile interpolates by a fractional weight; its Datetime64 arm inherits the same accuracy contract rather than inventing a stricter one for a single dtype.
- **Impact:** For datetime columns at modern timestamps, FP's mean/median can differ from pandas by up to ~128 ns. FP is the exact one in every such case. The divergence is invisible at second/day granularity, which is where the overwhelming majority of real datetime data sits.
- **Resolution:** ACCEPTED — deliberate, and consistent with precedent already in the tree. `nanptp`'s Timedelta64 arm carries the comment "Converting to f64 before subtracting loses ranges below one f64 ULP beyond 104 days", i.e. FrankenPandas had already rejected the f64 detour for temporal data; this extends the same choice to Datetime64. Reproducing pandas here would mean shipping a median that returns a timestamp the user never supplied. Same governing principle as DISC-018: where pandas is lossy, do the correct thing and document it rather than chase the defect.
- **Related, and NOT implemented:** pandas **raises** `TypeError: 'DatetimeArray' with dtype datetime64[ns] does not support reduction '<name>'` for datetime64 `sum`, `prod`, `var`, `sem`, and `skew`. FrankenPandas returns a missing value for these rather than inventing one; it deliberately does not add support pandas itself refuses.
- **`std` follows pandas exactly and is NOT part of this divergence** (br-frankenpandas-40ujm): pandas *does* support datetime64 `std`, returning a **Timedelta** — the dispersion of a set of instants is a duration. FP matches it bit-for-bit on the raw ns, including `ddof` handling (`n - ddof <= 0` → NaT) and pandas' toward-zero truncation of the sqrt. Verified: pandas returns the identical value for a Datetime64 series and a Timedelta64 series built from the same nanoseconds, which is why both families route through one computation. No exactness question arises here because a standard deviation involves a square root, so unlike mean/median there is no exact integer answer to prefer — FP and pandas are both computing in f64 and agree.
- **Tests affected:** `datetime64_averaging_reductions_match_pandas_adv58`, `datetime64_mean_median_round_toward_zero_adv58`, `datetime64_averaging_skips_nat_adv58`, `datetime64_averaging_is_exact_where_pandas_snaps_adv58` (all in `fp-types`). Beads: `br-frankenpandas-axhhk` (ordering reductions), `br-frankenpandas-adv58` (this).
- **Review date:** 2026-08-06

### DISC-020: STRICT mode deliberately reproduces pandas' silent timedelta int64 wrap
- **Reference:** pandas 2.2.3 wraps silently on the integer-multiplier and cumulative-sum timedelta paths — raw numpy int64 wraparound, no exception and no NaT. Probed: `Series([Timedelta.max]) * 2` → `-2` ns (displayed `-1 days +23:59:59.999999998`); `Series([max, max]).cumsum()` → `[9223372036854775807, -2]`; `Series([max, max, max]).cumsum()` → `[max, -2, max - 2]`, which shows the accumulator keeps wrapping rather than sticking. A **float** multiplier behaves differently: `Series * 2.0` returns NaT, so the wrap is integer-multiplier only.
- **Our impl:** under `OverflowPolicy::Strict` FrankenPandas **reproduces this exactly**, via `Timedelta::mul_scalar_with_policy` and `nancumsum_with_policy`. Rust's `wrapping_mul`/`wrapping_add` match numpy's int64 wraparound bit-for-bit, so no emulation is required. Under **every other policy — including the default `SurfaceNat`** — the result stays NaT, per DISC-018.
- **Impact:** this is the one place where FrankenPandas deliberately reproduces a behaviour it would otherwise classify as fail-open: multiplying a positive duration by 2 yields a *negative* duration presented as real data. That is the exact shape `br-frankenpandas-lgyy8` removed from this codebase. It is acceptable **only** because it is unreachable by default and requires a caller to explicitly ask for STRICT incumbent parity.
- **Resolution:** ACCEPTED — maintainer decision on `br-frankenpandas-fyr1z-wrap-signoff-s4lkx`, recorded verbatim: *"STRICT means bit-for-bit observable parity with the incumbent including its quirks, so reproduce the wrap and add a test that names it as a deliberately reproduced pandas behavior rather than a bug of ours."* This is a **port**, and STRICT's job is to be indistinguishable from the incumbent, not to be better than it. Note this deliberately differs in direction from DISC-019, where FrankenPandas chose exactness over pandas' f64 grid — the distinction is that DISC-019 concerns the *default* path, whereas this quirk is opt-in.
- **Tests affected:** `strict_reproduces_pandas_silent_timedelta_wrap_s4lkx` (pins the oracle values and states in its doc comment that a failure means pandas changed or a non-STRICT caller was misrouted — **not** that the arithmetic should be corrected), `non_strict_policies_still_refuse_to_wrap_s4lkx` (asserts the default and HARDENED still refuse to wrap, and that HARDENED still records the recovery).
- **Review date:** 2026-08-06

## Resolved Divergences

### DISC-005: Mixed string/numeric constructors now preserve pandas object semantics
- **Reference:** `pd.Series(["x", 1])` and `pd.concat([pd.Series(["x", 1])], axis=1)` preserve heterogeneous values under pandas `object` dtype
- **Our impl:** Constructor inference now uses the existing `Utf8` storage bucket for pandas-style object columns while preserving heterogeneous `Scalar` payloads in order
- **Impact:** `Series::from_values` and `DataFrame::from_series` now match pandas for mixed string/numeric constructor inputs
- **Resolution:** ACCEPTED - parity achieved and covered by live-oracle plus fixture-backed tests
- **Tests affected:** `live_oracle_series_constructor_mixed_utf8_numeric_reports_object_values`, `live_oracle_dataframe_from_series_mixed_utf8_numeric_matches_object_values`, `series_constructor_utf8_numeric_object_strict`, `dataframe_from_series_utf8_numeric_object_strict`
- **Review date:** 2026-04-15

### DISC-013: Series + Series union alignment does not sort the result index
- **Reference:** Pandas `Series.add(other)` (and `series + other` operator) on differently-indexed Series performs an outer-join alignment that returns a sorted result index by default.
- **Our impl:** RESOLVED - unique-label Series arithmetic now uses a sorted outer union for `+` / `-` / `*` / `/` and fill-value arithmetic, while preserving the duplicate-aware cross-product path tracked separately by DISC-014.
- **Impact:** The `FP-P2C-001 series_add_alignment_union_strict` fallback fixture has been refreshed to pandas 2.2.3 output: result index `[1, 2, 3]`, values `[NaN, NaN, 34.0]`.
- **Resolution:** FIXED in br-frankenpandas-cod1d13 by routing unique-label Series arithmetic through sorted union alignment in fp-frame instead of changing the generic fp-index discovery-order helper. NB: the listed strict test still fails today, but for a different root cause (DISC-011 nullable-Int64 dtype promotion); the sort-order issue this entry tracked is no longer present.
- **Tests affected:** `series_add_aligns_on_union_index`, `series_add_fill_sorts_unique_outer_union_index`, `FP-P2C-001/series_add_alignment_union_strict`.
- **Review date:** 2026-04-28

### DISC-014: Series + Series duplicate-label arithmetic Int64 promotion (prior WILL-FIX premise was incorrect)
- **Reference:** Pandas `Series + Series` with duplicate labels performs cross-product alignment per label. The result stays `int64` when every label matches on both sides (no unmatched pairing, so no NaN is introduced); it promotes to `float64` only when an unmatched label actually injects a NaN.
- **Our impl:** Matches pandas exactly — the duplicate-aware cross-product keeps `Int64` when no NaN is generated and promotes to `Float64` when alignment leaves a position unmatched.
- **Impact:** None. The earlier entry claimed pandas *always* promotes duplicate-label results to `Float64` even with no NaN; that is false. Verified against the pandas 2.2.3 live oracle: `Series([1,2,3], index=['a','a','b']) + Series([3,4,5], index=['a','a','b'])` returns `int64 [4,6,8]`, while a partial match (`index=['a','a']` + `index=['a','a','b']`) returns `float64` with a trailing `NaN`. The fixture `fp_p2c_001_duplicate_hardened.json` already expects `int64 [4,5]` and the conformance test `conformance_series_add_duplicate_labels` passes. The previously-proposed "always promote to Float64 when alignment *can* introduce NaN" fix would have *broken* parity for the fully-matched case and must not be implemented.
- **Resolution:** RESOLVED - no code change required; FP already matches pandas. This entry corrects the prior incorrect WILL-FIX premise, which conflated this case with the genuinely-open DISC-011 (Int64 column that actually *receives* a null). Oracle-verified 2026-06-01.
- **Tests affected:** `conformance_series::conformance_series_add_duplicate_labels` (passing).
- **Review date:** 2026-06-01

### DISC-016: RangeIndex set-operation result ordering (all four ops RESOLVED)
- **Reference:** pandas 2.2.3 `RangeIndex` set operations use different default `sort` semantics:
  - `union` / `difference` / `symmetric_difference`: `sort=None` — result is ascending-sorted EXCEPT when an operand is empty or the two operands are value-equal, where the surviving operand passes through unchanged (order preserved).
  - `intersection`: `sort=False` — result is ascending EXCEPT when BOTH operands are descending (`step < 0`), where it is descending. (Verified vs pandas 2.2.3 over 150k random pairs.)
- **Our impl:** RESOLVED for all four. Previously fp returned every set op in self/discovery order (matching pandas only where the affine fast paths happened to already be in pandas' order), diverging for both-non-empty operands with descending or non-aligned (interleaving) lattices — union/difference/symmetric_difference for ~any descending/interleaved input, and intersection for self-descending ∩ other-ascending (≈23.5% of multi-element intersections). fp now normalizes operands to their ascending equivalent so the fast paths and fallbacks produce pandas' order, sorts the interleaving union/symmetric_difference fallbacks, and routes both-descending intersection through the operands as-is (self-order yields descending). Empty-operand and value-equal passthrough preserved; lazy affine / two-affine-run backing retained for the aligned common case (only reordered cases materialize).
- **Impact:** all four ops now bit-match pandas across a 263-case randomized differential (descending, non-aligned, empty, value-equal, disjoint, subset), plus 150k-pair Python sweeps confirming the ordering rules.
- **Resolution:** FIXED (DustySummit, br-frankenpandas). union/difference/symmetric_difference in commit 88e2a8487; intersection ordering in the follow-up commit.
- **Tests affected:** `range_index_set_ops_match_pandas_2_2_3_differential_dustysummit` (data-driven from `testdata_rangeset_pandas_cases.rs`, all four ops); `range_index_set_ops_use_direct_values_b7nxg`, `range_index_set_ops_closed_form_membership_preserves_order_iatnc`, `range_index_set_ops_return_affine_spans_iatnc` (refreshed to pandas-correct ordering).
- **Review date:** 2026-07-23

### DISC-017: RangeIndex.slice_locs / searchsorted reject monotonic-decreasing (step < 0) ranges
- **Reference:** pandas 2.2.3 permits `RangeIndex(10,0,-1).slice_locs(8,3)` → `(2, 8)` and `RangeIndex(10,0,-1).searchsorted(5)` → `0` on descending ranges.
- **Our impl:** ACCEPTED divergence — fp returns an explicit `InvalidArgument` error for `slice_locs` and `searchsorted` when `step < 0` ("requires a monotonic[ally-]increasing RangeIndex"), rather than replicating pandas' behavior. get_loc / get_indexer / get_indexer_non_unique / reindex (direction-agnostic value→position lookups) DO work correctly on descending ranges; only the two ordering-boundary ops refuse.
- **Impact:** Rationale for refusing rather than matching: pandas' `searchsorted` on a descending index is a numpy artifact — numpy `searchsorted` assumes an ascending array, so `desc.searchsorted(5) = 0` is a meaningless insertion point, not a correct one. pandas' descending `slice_locs` inherits the same ascending-assuming `get_slice_bound`, producing quirky edge results (e.g. `[7].slice_locs(8,-9) = (1,0)` — an EMPTY slice for a value that is in `[-9, 8]`). A randomized 40k-pair differential found no simple closed form matching pandas' descending slice_locs (≈7.6% of a naive `value<=start` / `value<end` closed form diverged, all in boundary/single-element cases). fp's explicit error avoids silently reproducing these numpy quirks; callers needing a descending slice can reverse the index first.
- **Resolution:** ACCEPTED (DustySummit, 2026-07-23). Revisit only if a use case needs pandas-bug-compatible descending `slice_locs`; it would require replicating pandas' `get_slice_bound` direction handling, not a closed form.
- **Tests affected:** none (fp's error path is covered by existing RangeIndex tests; no descending slice_locs/searchsorted parity test is asserted).
- **Review date:** 2026-07-23

### DISC-018: `oracle_attestation` — what a provenance stamp on an EXPECTED-ERROR fixture claims
- **Reference:** `fixture_provenance.oracle_script_sha256` normally asserts *"running this oracle script on this input reproduced these expected VALUES"*. An expected-error fixture pins no values, so the same stamp cannot mean the same thing there.
- **Our impl:** error fixtures whose refusal came from pandas now carry an explicit `fixture_provenance.oracle_attestation: "error_agreement"`, meaning only *"running this oracle script on this input made PANDAS raise too"*. **Absence of the key keeps the strong value-reproduction reading**, so nothing about the other 977 stamps changed. The message text is deliberately NOT part of the claim: `expected_error_contains` pins FrankenPandas's wording (checked by the Rust harness) while the oracle surfaces pandas' own English, and the two were never meant to match.
- **Impact:** 49 of the 85 expected-error fixtures qualify. The other **36 are deliberately left unstamped and still count as red**: 31 were refused by the oracle *adapter's own argument validation* (e.g. `dataframe_concat concat_axis must be 0 or 1, got 2`) and 5 escaped as `unexpected`, so **pandas was never invoked** and there is no agreement to attest — the claim would be true and vacuous. The split is structural, from `pandas_oracle.oracle_error_origin` (an `OracleError` raised `from exc` wrapped an engine call; a bare one is adapter validation), never a substring match on the message. The 31 adapter refusals are better read as oracle coverage gaps.
- **Resolution:** ACCEPTED (BlueRobin, 2026-08-08, br-frankenpandas-fixture-corpus-stale-vs-oracle-p6srr). The alternative — restamping all 85 under the bare key — was rejected as silently widening what `oracle_script_sha256` means corpus-wide to make a number go down. `scripts/check_fixture_freshness.sh` is untouched and reads only the three oracle keys via `.get()`, so a provenance **superset is already gate-legal**; that is also why the generator fixture `fp_generated_tn6qb2_...` now restamps in place with its `generation_command` / `input_matrix` / `intentional_divergence_notes` preserved instead of being refused.
- **Tests affected:** `oracle/tests/test_error_origin.py` (7 cases: origin classification, fail-closed default, provenance on the error path); `oracle/tests/test_regenerate_fixtures.py` (superset faithful / dropped-extras refused / changed-extra refused / stale-sha refused / undeclared-key refused / attestation required / text insertion).
- **Review date:** 2026-08-08

### DISC-019: the oracle builds NULLABLE dtypes for int+null / bool+null payloads, unlike pandas' own constructor
- **Reference:** pandas 2.2.3 infers `pd.Series([1, None, 3])` as **float64** (values `[1.0, nan, 3.0]`) and `pd.Series([True, None])` as **object**. Nullable `Int64` / `boolean` are reached only by asking for them explicitly.
- **Our impl:** `pandas_oracle.series_dtype_for_payload_values` returns `"Int64"` for an all-int payload containing a null, and `"boolean"` for an all-bool payload containing a null — so the oracle constructs a column pandas' own constructor would never build from the same data.
- **Impact:** ACCEPTED, and it is **load-bearing rather than a defect**, which is the opposite of how it reads. The fixture format tags **every value** with its own `kind`; a float64 column would rewrite each `{"kind":"int64"}` into `{"kind":"float64"}` on the way out, so the nullable dtype is what preserves the payload's kinds across the round trip. Measured 2026-08-08 over the whole corpus, switching both arms to pandas' inference: `agree` 977 → 947 (−30), `moved, unattributed` 151 → 181 (+30), and the `KIND int64->float64` move class 57 → 86 (+29). The change makes the corpus strictly worse and **grows the very class it was expected to shrink**.
- **Consequences worth knowing:** (a) an int+null column is nullable `Int64`, so `fillna` with an incompatible scalar RAISES rather than promoting to object — that is why `fp_p2d_050_dataframe_fillna_cast_error_strict` records an error pandas-native would not produce, and why the fillna half of `br-frankenpandas-fp-stricter-than-pandas-rejections-gtkz1` cannot be settled by relaxing FrankenPandas alone; (b) `kinds <= {bool,int64,float64}` returns `float64`, while pandas infers **object** for a genuine `bool`+`int` mix (`pd.Series([True, 2])`), a third arm with the same shape and no fixture exercising it.
- **Resolution:** ACCEPTED (BlueRobin, 2026-08-08, br-frankenpandas-9ooer). The real question is not "which dtype should the oracle pick" but **whether the fixture format should carry a column dtype instead of per-value kinds** — the current format cannot express pandas' constructor promotion at all, and the dtype forcing is the compensation. Re-opening this should start from that, not from the dtype table.
- **Tests affected:** none changed. The negative result is recorded in `series_dtype_for_payload_values`'s own docstring so the next reader does not repeat the experiment.
- **Review date:** 2026-08-08

### DISC-020: `str.index` / `str.rindex` report a missing position where pandas raises — and therefore do NOT take the float64 promotion
- **Reference:** every OTHER int-returning `.str` accessor follows a three-way result-dtype rule, measured on live pandas 2.2.3 over `pd.Series([...], dtype=object)`: no gaps → `int64`; a `None`/`nan`/non-string element → **`float64`** with the gap as `nan`; a `NaT` → **object**, ints unpromoted and the `NaT` preserved, because float64 cannot hold a NaT. `pd.Series.str.index(sub)` does not participate: it **raises `ValueError: substring not found`** rather than returning anything for an absent needle.
- **Our impl:** `StringAccessor::index_of` / `rindex_of` return a missing value at an absent needle instead of raising — an FP-defined nullable-int contract for an operation pandas has no total equivalent of. Because they are the only int-returning accessors that can invent a gap from *present* input, applying the promotion rule to them would make a found-position `float64` purely because some other row's needle was absent. They therefore keep `Int64` at every found position with only the gaps `NaN`. The oracle already models exactly this (`pandas_oracle._index_of_result` builds an OBJECT series of ints-and-NaN and says so in its docstring); this entry records the same contract on the FrankenPandas side so the two cannot drift apart silently.
- **Impact:** two accessors, `str.index` / `str.rindex`. Their sibling `str.find` / `str.rfind` are unaffected — pandas defines those totally (`-1` when absent), they never invent a gap, and they do take the promotion rule. The other ten int-returning accessors (`len`, `find`, `rfind`, `find_with_bounds`, `rfind_with_bounds`, `encode`, `count`, `count_literal`, `count_matches`, `split_count`) are on the pandas rule via `StringAccessor::apply_str_int_scalar`.
- **Resolution:** ACCEPTED (WindyHare, 2026-08-08, `br-frankenpandas-lwvet`). The alternative considered and rejected was routing these two through the promoting helper for uniformity: it would have redefined an already-documented FP contract as a side effect of a pandas-parity change, and moved `fp_p2d_299` / `fp_p2d_300` further from the oracle rather than closer.
- **Tests affected:** `str_int_returning_ops_promote_to_float64_unless_the_null_is_nat` (fp-frame, final block pins the exception); fixtures `fp_p2d_299_series_str_index_of_null_hardened`, `fp_p2d_300_series_str_rindex_of_null_hardened`.
- **Review date:** 2026-08-08

## Rules

1. Every divergence gets a sequential ID (DISC-NNN)
2. Must state whether ACCEPTED, INVESTIGATING, or WILL-FIX
3. Must list affected test cases
4. Must include review date
5. Tests for ACCEPTED divergences use XFAIL markers where applicable
