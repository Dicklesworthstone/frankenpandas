# fp-frame UBS Inventory Triage - 2026-06-17

Scope: `br-frankenpandas-yavyk`, the broad UBS backlog for
`crates/fp-frame/src/lib.rs`.

## Evidence

The originating scan for `br-frankenpandas-fbav3` completed UBS on
`crates/fp-frame/src/lib.rs` with a broad pre-existing whole-file inventory:

- 583 critical
- 48,203 warning
- 3,947 info

The same note recorded that UBS internal fmt, clippy, check, and test sub-gates
were clean. The issue classes were broad static heuristics rather than one
regression class: panic/test heuristics, semantic comparisons that look like
secret comparisons, direct indexing, clones, casts, and similar file-wide
findings.

Later touched-file attempts against `fp-frame` often did not reach findings at
all. Multiple artifacts show UBS timing out or stalling during the Rust
`ast-grep` unwrap scan for the large one-file crate. Examples:

- `tests/artifacts/perf/lavender_s2i37_vc_ubs_touched.txt`: terminated after
  more than five minutes while running the Rust `ast-grep` unwrap pass.
- `tests/artifacts/perf/x8mdu_ubs_fp_frame.txt`: `ubs_exit=124` after timeout.
- `tests/artifacts/perf/uza0490_ubs_fp_frame.txt`: reached "Scanning rust..."
  and did not emit findings before timeout.

## Triage

This is not one fixable defect. It is an overloaded broad scan across the
load-bearing `fp-frame` implementation file. Treating the whole inventory as a
per-commit blocker is counterproductive: it hides new regressions behind a huge
known baseline and causes narrow feature/performance commits to stop on
unrelated scanner debt.

The actionable split is:

1. Keep `ubs <changed-files>` mandatory for ordinary changed files.
2. For `crates/fp-frame/src/lib.rs`, run UBS with a bounded timeout and record
   whether it emitted new focused findings or only reproduced the known broad
   inventory/stall behavior.
3. Do not mix whole-file UBS remediation into narrow feature or performance
   beads.
4. When a specific UBS class is ready to remediate, file a narrow bead for that
   class and validate it with focused tests plus a bounded UBS rerun.

## Current policy

For commits touching `crates/fp-frame/src/lib.rs`, this is the expected gate:

```bash
timeout 180s ubs crates/fp-frame/src/lib.rs
```

Passing UBS remains a hard pass. A timeout or reproduction of the documented
whole-file inventory is acceptable only when the commit has passed the relevant
crate build/test gates and the closeout cites this audit. New focused UBS
findings on the touched hunk still need to be fixed before commit.

Suggested follow-up beads should be narrow, for example:

- direct indexing in non-test hot paths
- `unwrap`/`expect` outside tests and fixture setup
- cast checks in numeric conversion helpers
- semantic-comparison false positives that need UBS suppression support

## Current validation

Commands run from `/data/projects/.scratch/frankenpandas-cod-a-toyzp` on
2026-06-17:

- `timeout 120s ubs AGENTS.md artifacts/audits/fp_frame_ubs_inventory_2026-06-17.md .beads/issues.jsonl`:
  exit 0; UBS reported no recognizable code languages for the changed
  documentation and bead metadata files.
- `timeout 180s ubs crates/fp-frame/src/lib.rs`: exit 124; no findings were
  emitted before timeout, reproducing the documented large-file Rust scan stall.
- `RCH_REQUIRE_REMOTE=1 CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo check -p fp-frame --all-targets`:
  blocked before compilation because RCH found no admissible remote worker
  (`critical_pressure=2,hard_preflight=9`) and refused local fallback. The same
  result persisted after `rch workers capabilities --refresh`.
