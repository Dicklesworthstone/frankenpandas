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
