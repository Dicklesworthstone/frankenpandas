# br-frankenpandas-vrjrf — reusable worker pool for the string affix kernels

**Status: code landed (`b19f8521d`). NO PERF CLAIM — every row is
`NULL_UNDECIDABLE`, and the bead's own success criterion ("the A/A null MUST
pass — that is the point") is NOT met.**

Agent: LavenderPine (claude-code / opus-5). Date: 2026-08-05.

## What landed

`startswith`/`endswith` over a contiguous Utf8 column now run on a
process-wide worker pool instead of opening a fresh `std::thread::scope` per
call. The pool is a lazy `OnceLock` built on first use of the parallel arm,
which already requires a ≥8 MiB buffer.

Two things this settles independently of any measurement:

1. **A reusable pool here needs NO unsafe.** The standing note in the
   parallel-overhead ledger says otherwise. `thread::scope` is required only
   because `as_utf8_contiguous` returns BORROWED `(&[u8], &[usize])`. The
   backing already holds `Arc<[u8]>`/`Arc<[usize]>`, and
   `as_utf8_contiguous_arc` (landed `1425376b2`, same bead) returns owned
   clones for two refcount bumps. Owned `Arc<[T]>` is `Send + Sync + 'static`,
   so a `'static` worker holds it directly. `#![forbid(unsafe_code)]` intact.

2. **The generic bound did not have to change.** Pool jobs must be `Send +
   'static`; `F: Fn(&[u8]) -> bool` cannot be, because callers capture a
   borrowed `pat: &str`. Rather than tighten that bound and force all 15
   predicate call sites to own their captures, the two affix predicates — the
   only ones whose capture is a single needle — own it as `Arc<[u8]>` and
   carry a `StrAffix` enum. `apply_str_bool` / `apply_str_bytes_bool` are
   untouched.

## Why the lever was attempted

`br-frankenpandas-2s5hs` eliminated the competing explanation. Halving the
worker count moved the A/A null FURTHER from unity and cost 1.333x the time,
so worker count is not the mechanism. With the forced-serial control (no
spawn) passing its null at 0.9992, the spawn itself was what remained.

## Measurement: THREE clean runs, NOTHING decidable

Arms built from trees differing only in `crates/fp-frame/src/lib.rs`:

| arm | ELF SHA-256 | `affix_rows` symbol | `fp-str-` symbol |
|---|---|---|---|
| reference (per-call `thread::scope`) | `a72c7541789e64c1…` | 0 | 0 |
| candidate (pooled) | `b7130bfa196d241a…` | 1 | 1 |

Distinct SHA-256 **and** distinct symbol content, retiring the
`rch-ab-elf-retrieval-trap` false-REJECT class. The candidate ELF was rebuilt
after a late rustfmt fix and its SHA changed (`dbbe2979…` → `b7130bfa…`) —
worth recording, because the pre-rebuild binary would have measured a source
that is not HEAD.

Host `frankenlibc-test`, 10 cores, `str_startswith_arrow` @1M, candidate +
reference + live pandas in ONE invocation, exec order `[reference,
candidate]`, pandas 2.2.3 / pyarrow 24.0.0.

| run | cand p50 | cand null | ref p50 | ref null | **pandas null** | cand/ref |
|---|---|---|---|---|---|---|
| A    | 2226.9 µs | 0.9421 | 2186.6 µs | 0.8775 | **0.9738** | 0.98 |
| rep3 | 1216.7 µs | 1.0806 | 2451.6 µs | 1.0019 | **0.9778** | 2.01 |
| rep6 | 1343.5 µs | 1.0224 | 3822.2 µs | 1.1822 | **0.9836** | 2.85 |

Verdict on every row: `NULL_UNDECIDABLE`.

### The headline is the control column, not the candidate

**pandas' own A/A null failed all three runs** (0.9738 / 0.9778 / 0.9836
against a ±2% limit). pandas' arrow `startswith` is single-threaded and was
the stable control in every earlier session — when it fails, the host is not
in a state where ANY row on this workload can be decided, whatever the
candidate does. Note this is workload-specific: on the same box hours earlier
`str_contains_arrow` had pandas' null at 0.9866 (PASS). So
`frankenlibc-test` — the only quiet box on the 11-worker fleet — currently
cannot decide `str_startswith_arrow` at all.

The spreads confirm it. Identical binaries, three runs: the candidate's p50
spans 1216–2227 µs (1.83x) and the reference's spans 2187–3822 µs (1.75x).
`cand/ref` spans 0.98–2.85. No effect of the size being looked for survives
that.

### What can and cannot be said

**Cannot:** that the pool is faster. Two of three runs put the candidate at
~half the reference's time and its p50 never reached the reference's minimum
in those runs, but one run shows parity and the gate rejects all three.

**Cannot:** that the pool fixes the A/A null. It does not — 0.9421 / 1.0806 /
1.0224, none inside ±2%. The bead required this to pass and it did not.

**Can:** that the pool does no measurable harm, and that it is bit-transparent
and correct (below).

## Correctness evidence (the basis for landing)

Landed on correctness only, following the `br-frankenpandas-i7znp` precedent
of keeping an `UNDECIDABLE` change in-tree with no speedup attached.

- `cargo test -p fp-frame --lib` — 3199 passed / 0 failed (+2, none lost)
- `cargo clippy -p fp-frame --lib -- -D warnings` — exit 0
- `cargo fmt -p fp-frame -- --check` — clean
- `ubs crates/fp-frame/src/lib.rs` — see below

Bit-transparency: same needle bytes, same `starts_with`/`ends_with` over the
same row ranges, same chunk boundaries (identical `workers` formula and
`n.div_ceil(workers)`), parts merged in submission order, and one shared
`affix_rows` used by both the serial and pooled arms so they cannot drift.

Two new tests, both sized to actually reach the pooled arm — a small fixture
would exercise only the serial branch and prove nothing:

- `str_affix_pooled_matches_serial_oracle_vrjrf` — 300k rows / ~9.6 MB (over
  the 8 MiB gate, ≥2 workers), asserted per row against `str::starts_with` /
  `str::ends_with` for five needles including empty and no-match. Catches an
  off-by-one chunk boundary, a dropped part, or parts merged out of order.
- `str_affix_pool_is_safe_under_concurrent_callers_vrjrf` — four threads, four
  corpora, needles chosen so a cross-delivered part surfaces as a wrong value
  instead of being masked by two callers agreeing. A pool that ran jobs while
  holding the receiver lock hangs here.

### UBS

`crates/fp-frame/src/lib.rs` is 184k lines and UBS's own `UBS_MODULE_TIMEOUT`
(300s, independent of the outer `timeout`) fires long before a verdict — this
is the documented large-file condition (`br-frankenpandas-yavyk`,
`artifacts/audits/fp_frame_ubs_inventory_2026-06-17.md`). Re-run with
`UBS_MODULE_TIMEOUT=2400` it completes: 997 critical / 56754 warning / 7248
info, the known whole-file inventory.

Exactly one of the 55 printed locations fell in a changed range — `lib.rs:41866`,
a `panic!` in the pool's send-failure path, under the file's pre-existing
"panic! macro(s) present" category (106 instances). **Fixed rather than
waived**: `mpsc::SendError` hands the job back, so the pool now runs it on the
caller's thread instead of panicking. That is strictly more robust than the
panic it replaces — the failure mode degrades to caller-side execution rather
than aborting — and it removes a `panic!` from library code.

## Retry predicate

Re-measure ONLY on a host where **pandas' A/A null passes ±2% on
`str_startswith_arrow`**. That is the precondition this session could not
meet, and without it no candidate number means anything. Concretely:

1. Confirm the control first: run the reference arm and pandas alone and
   require `pandas_within_limit = true` before spending a candidate run.
2. Then the decision rule the bead already set: the pool is a WIN only if the
   candidate's own A/A null lands within ±2% AND `candidate_vs_reference`
   clears the two-x null margin. Neither held here.
3. If a decidable host cannot be found, the honest next step is not another
   re-run — it is to make the FP-side spread smaller or the measurement
   longer, because the gate is behaving correctly and the signal is genuinely
   below the noise.

If a decidable host shows the pool at parity or worse, **revert it**: it costs
~200 lines of concurrency machinery and a process-lifetime thread commitment
that only a real win justifies.

## Method notes for the fleet

- **Do not poll the bench host while a row is running.** An interactive `ssh
  … tail/uptime/ps` is enough to trip the host-wide 20%-busy gate; one run
  aborted at `invocation_postflight` with `busy=[2,6]` for exactly this.
- **Let a build settle before measuring.** A run started immediately after a
  release build aborted at `post_measurement` with `busy=[4,6]` — cargo/rustc
  teardown and ELF writeback were still in flight. A ~90 s pause fixed it.
- The gate is flaky-but-achievable on a shared box: 5 of 8 attempts here were
  blocked by transient single-CPU noise. Retrying until a clean window is not
  gate-weakening — the gate is a precondition on host state, and the reported
  result is whatever the harness computes once it opens. **No gate, flag, or
  threshold was modified at any point.**
