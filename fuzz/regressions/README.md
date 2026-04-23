# Fuzz Regression Corpus

This directory holds **minimized** crash artifacts that must always
reproduce the pre-fix crash behavior (crash before the fix, clean run
after). Each minimized artifact has a matching `#[test]` in
`crates/fp-conformance/tests/fuzz_regressions.rs`.

Tracked under br-frankenpandas-lvl6.

## Why

fuzz/corpus/<target>/ holds working-set inputs the fuzzer mutates. Those
grow and shrink as the fuzzer finds new coverage paths. Corpus entries
are NOT guaranteed to be minimal or to correspond to specific crashes.

`fuzz/regressions/<target>/` is different: every file in here is a
minimized crash input paired with a regression test that replays it.
The file + test together lock in the fix for a specific historical
bug. A future refactor that reintroduces the bug fails the regression
test immediately.

Per /testing-fuzzing skill Rule 10: *"Every crash artifact becomes a
regression test or it WILL regress. Untested fixes are temporary fixes."*

## Workflow (when a fuzz target crashes)

1. Run `cargo +nightly fuzz tmin <target> <artifact>` to minimize
   the crashing input.
2. Rename the minimized file to
   `fuzz/regressions/<target>/<short_description>_<YYYY_MM_DD>.<ext>`
   (e.g. `fuzz_csv_parse/unterminated_quote_2026_05_01.csv`).
3. Add a matching `#[test]` to
   `crates/fp-conformance/tests/fuzz_regressions.rs`:
   ```rust
   #[test]
   fn regression_fuzz_csv_parse_unterminated_quote_2026_05_01() {
       let input = include_bytes!(
           "../../../fuzz/regressions/fuzz_csv_parse/unterminated_quote_2026_05_01.csv"
       );
       let _ = fp_conformance::fuzz_csv_parse_bytes(input);
   }
   ```
4. Verify the test REPRODUCES the crash on the pre-fix commit
   (optional but recommended for high-severity crashes — gives
   historical receipt of the bug).
5. Land the fix.
6. Verify the regression test now PASSES on the post-fix commit.
7. Commit both the regression artifact + the test + the fix in the
   same PR. Commit message: `fix(<crate>): reject X (br-...)` with
   a `Regression: fuzz/regressions/<target>/<name>` footer.

## File layout

```
fuzz/regressions/
├── README.md                           (this file)
├── fuzz_csv_parse/
│   ├── unterminated_quote_2026_05_01.csv
│   └── quoted_newline_oom_2026_06_12.csv
├── fuzz_parquet_io/
│   └── rowgroup_count_overflow_2026_07_30.parquet
└── ...
```

One directory per fuzz target name. Filenames carry a short
semantic tag + YYYY_MM_DD date.

## CI

`cargo test -p fp-conformance --test fuzz_regressions` runs on every
PR via the existing `test` job (br-frankenpandas-ffhs). Fast:
include_bytes! + one parse call per regression = sub-second for
hundreds of entries.

## When this directory is empty

No historical crashes have been locked in yet. The first real crash
from the fuzz-regression CI job (br-zjme) populates a file here +
a test next to it.

## Related beads

- `br-zjme` (fuzz CI regression corpus, closed) — the CI job that
  produces crashes this dir absorbs.
- `br-i9rj` (cargo fuzz cmin/tmin cadence, open) — the
  minimization workflow that produces the artifacts committed
  here.
- `br-auys` (memory/time caps, open) — prevents OOM kills from
  masking real bugs in the regression tests.
- `br-lvl6` (THIS bead) — infrastructure scaffolding.
