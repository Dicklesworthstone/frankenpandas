# Fuzz Regression Workflow

`fuzz/` is the persistent regression corpus for every libFuzzer target in this repo.

Workflow:
- reproduce the crash with `cargo fuzz run <target> fuzz/corpus/<target>`
- minimize it with `cargo fuzz tmin <target> <crash-path>`
- move the minimized input into `fuzz/corpus/<target>/`
- commit the corpus update with the bug fix
- let CI replay the committed corpus on every push and pull request

Conventions:
- every target in `fuzz/fuzz_targets/` must have `fuzz/corpus/<target>/`
- keep at least four committed seeds per target: empty/minimal, valid/happy path, malformed boundary, and shape-stretching boundary
- keep `fuzz/artifacts/<target>/README.md` in git; new crash files are written there locally and by CI, then minimized into `fuzz/corpus/<target>/`

CI:
- `.github/workflows/ci.yml` runs the `fuzz-regression` job on every push and pull request
- `.github/workflows/fuzz-nightly.yml` runs longer nightly campaigns and uploads any new crash artifacts
- `.github/workflows/fuzz-minimize.yml` runs `cargo fuzz tmin` / `cmin` weekly (Sunday 07:00 UTC) per br-i9rj

Dictionaries (`-dict=` on the libFuzzer command line):
- `fuzz/dictionaries/csv.dict` — CSV / RFC 4180 delimiters + quoting + BOM
- `fuzz/dictionaries/json.dict` — JSON literals + escapes + numeric edges + deep-nesting seeds
- `fuzz/dictionaries/excel.dict` — ZIP magic + xlsx XML fragments + XLS compound-file magic
- `fuzz/dictionaries/parquet.dict` — PAR1 magic + Thrift type/codec/encoding tokens
- `fuzz/dictionaries/arrow_ipc.dict` — ARROW1 + record-batch metadata + type tokens

Attach the relevant dictionary to any manual `cargo fuzz run` invocation, e.g.
`cargo fuzz run fuzz_csv_parse corpus/fuzz_csv_parse -- -dict=dictionaries/csv.dict`.
Per /testing-fuzzing Rule 4, structure-aware dictionaries typically accelerate coverage
5-50x on structured-format parsers.

Sanitizers:
- cargo-fuzz defaults to AddressSanitizer (ASan). Per /testing-fuzzing Rule 5, prefer
  ASan + UBSan together. Invoke explicitly under nightly:
  `RUSTFLAGS="-Zsanitizer=address,undefined" cargo +nightly fuzz run <target>`.
- MSan / TSan are separate campaigns (ASan + MSan are incompatible). Ship-ready crash
  artifacts from any sanitizer campaign belong in `fuzz/regressions/` (see br-lvl6).
