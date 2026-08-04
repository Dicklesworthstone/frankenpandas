# Element-wise float map: the worker loop was scalar with a per-element bounds check

**Agent:** cod-pandas (claude-code / opus-5)
**Date:** 2026-08-01
**Host:** `thinkstation1`, AMD Ryzen Threadripper PRO 5975WX, 32 physical / 64 logical,
kernel `6.17.0-35-generic`
**Base commit:** `f4fc22775ca8ee24d922794dcdeefef9b5d22a7f`

Decision: **KEEP.** Bit-identical, routing-proven, strictly fewer instructions for
the same memory traffic and the same thread count. Measured **1.0721x** FP-side.

⚠️ **NOT gate-admissible, and no vs-pandas claim is made.** The effect fails the
corrected null gate's clause 2 (see below). It is kept because it is a
strictly-better lowering of the same loop, not because the timing separated.

## Why this operation

`docs/NEGATIVE_EVIDENCE.md` records `sqrt` (0.805x) and `log` (0.630x) at 1M
Float64 as the only surviving measured losses against live pandas 2.2.3. The
2026-07-31 fused-witness entry left a concrete retry predicate: reopen `sqrt`
only after a profile names the largest remaining source-parallelizable residual.
This is that profile.

## The defect

`perf annotate` of the shipped worker symbol in
`par_map_vec_f64_with_witness::<…sqrt…>` (ELF
`41a37287914db9e7386ac36e94eb626a79ecd8356746c3b369fc8ce555b1ba1b`):

```
  4.45 : lea    (%r15,%rcx,1),%r8
  3.80 : mov    0x10(%r11),%rax
  5.10 : cmp    %rax,%r8            <- per-element bounds check
  0.00 : jae    <panic>
  3.95 : mov    0x8(%r11),%rax
  6.43 : movsd  (%rax,%rcx,8),%xmm2 <- ONE f64 per iteration
  3.63 : movsd  %xmm1,(%r10,%rcx,8)
```

The kernel took a by-index closure `g(base + j)` that indexes a *captured* slice.
LLVM cannot prove those accesses stay in bounds, so it keeps a bounds check per
element and never vectorizes: the hottest loop in the whole operation moved one
`f64` at a time, in SSE scalar encodings, on a machine with AVX2.

Whole-ELF instruction counts confirm the shape rather than the intent:

| ELF | `sqrtpd` (packed) | `sqrtsd` (scalar) |
|---|---:|---:|
| baseline `41a37287…` | 2 | 64 |
| candidate `93b5f2bd…` | **5** | 81 |

The three new packed sites include
`par_map_slice_f64_with_witness<f64, …sqrt…>` at `0x68fe40` and the serial
fallback in `typed_float_unary_nullable_owned_par::<sqrt>` at `0x686200` —
neither of which exists in the baseline. That is the routing proof.

## The change

`par_map_slice_f64_with_witness` hands each worker its own **input slice**
instead of a base index, so the value loop is a bounds-check-free slice zip, and
derives the validity word per 64-value block while those values are still in L1.
The two contiguous all-valid arms (`as_f64_slice`, `as_i64_slice`) route to it;
the nullable arms keep the by-index kernel, whose per-element validity lookup is
inherently indexed.

Bit-identity: `f` is unchanged and per-index; bit `k` of word `b` is set iff
`!f(x).is_nan()` for chunk offset `b * 64 + k`, which is exactly the old
`words[j / 64] |= 1 << (j % 64)` rule; `all_valid`/`all_finite` remain boolean
AND reductions. `sqrtpd` and `sqrtsd` are both IEEE-754 correctly-rounded, so
packing does not move a bit. The measured output checksum is `e700f53534db5c6d`
in **both** arms — identical to the checksum in the 2026-07-31 entry.

## Measurement (interleaved, order alternated, A/A null control)

Two ELFs built locally from the exact base, differing only in
`crates/fp-columnar/src/lib.rs`. 12 rounds, order alternated per round, with a
candidate-vs-candidate A/A control at the same cadence.

| arm | p50 over rounds |
|---|---:|
| baseline `41a37287…` | 1881.7 us |
| candidate `93b5f2bd…` | 1768.4 us |

- Effect median ratio **1.0721x**, bootstrap 95% CI **[1.0029, 1.1834]**
- A/A null median **0.9811x**, CI **[0.9306, 1.0150]**
- clause 1 (effect CI excludes unity): **pass**
- clause 2 (|effect − 1| > 2 × null half-width): **FAIL** — 0.0721 vs 0.0844
- clause 3 (A/A within 2% of unity): **pass** (0.9811)

Host one-minute load averaged **~42** across the window (peer agent builds and a
peer `redis-benchmark`). The gate is not relaxed and no competitive row is
claimed; this is banked as a sub-gate maintenance result.

## Why this cannot flip `sqrt` on its own — the real residual, measured

An interleaved round-robin probe under the same allocator fp-bench uses
(mimalloc), 1M f64, 30 rounds, each arm timed once per round in rotating order:

| cost, per call | p50 | does pandas pay it? |
|---|---:|---|
| `thread::scope` spawn+join, 8 empty threads | **396.9 us** | **no** — pandas is single-threaded |
| `vec![0.0; 1M]` (allocator zero-fill, then 100% overwritten) | **206.1 us** | **no** — numpy allocates `empty`, never zeroes |
| `Vec::with_capacity(1M)` (no zero-fill) | 0.8 us | — |
| whole shipped `sqrt` call | ~1002–1390 us | pandas: 1078 us |

**Over half of a 1M `sqrt` call is per-call fixed overhead that the incumbent
does not pay at all**, and the larger half is **OS thread creation**, not the
map. The map itself was the part this commit fixed; that is why the effect is
~7% and not ~2x.

`rg -c "thread::scope" crates/ -g '*.rs'` finds **89 call sites** across
fp-frame (59), fp-columnar (13), fp-join (9), fp-io (5), fp-index (2). There is
no thread pool anywhere in the tree. Every parallel operation in FrankenPandas
pays fresh `clone(2)` per worker per call.

## Concrete retry predicates

- **`sqrt`/`log` vs pandas** stays open until the ~397 us per-call thread-creation
  cost is removed. A persistent worker pool is the fix, but a pool that runs
  closures borrowing non-`'static` data (which every one of these kernels needs —
  they borrow `&mut [f64]` output chunks) cannot be built in safe Rust; that is
  why `rayon`/`crossbeam` use `unsafe` internally. `crates/fp-columnar/src/lib.rs`
  is `#![forbid(unsafe_code)]` and the project directive keeps it. **This is a
  policy decision, not an engineering one, and it is escalated rather than
  worked around.** Options: accept a vetted pure-Rust dependency, or carve a
  single audited `unsafe` scoped-pool module out of the forbid.
- **The 206 us zero-fill** has the same blocker: obtaining `&mut [f64]` over
  uninitialised memory for parallel workers requires `unsafe` (`set_len`). Every
  safe alternative measured worse — per-worker `collect` + concat replaces one
  8 MB zero-fill with a 16 MB serial memcpy.
- Do **not** retry a wider worker cap without first removing the spawn cost:
  more workers buys more bandwidth and more `clone(2)` at the same time.

## Validation

- `cargo test -p fp-columnar --lib`: **596 passed, 0 failed**, 57 ignored.
- `cargo test -p fp-conformance`: **419 + 2 + 1 + 2 passed, 0 failed**.
- New test `slice_par_witness_all_valid_input_producing_nan_matches_scalar_reference`
  pins the arm this commit created. The pre-existing boundary test could not
  reach it: its NaN-heavy columns carry input gaps, so `as_f64_slice` bails and
  they take the nullable by-index arm, and its one all-valid column is NaN-free —
  which leaves `bits` full and `all_valid` true however the packing is written.
  The new test uses all-valid inputs whose *output* is NaN, at n = 200_003 /
  262_145 / 393_281 (ragged final chunk and ragged final validity word), with the
  NaN-producing rows forced onto word boundaries k = 0 and k = 63, and asserts
  non-vacuity.
