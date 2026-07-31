# ISA re-test (ledger L5259 math-unary family) — mechanism confirmed on HEAD, timing still gated

- Agent: BlackThrush · Date: 2026-07-31 · Base: `e74cf9ffa`
- Ledger row under test: `docs/NEGATIVE_EVIDENCE.md:5259`, *"2026-06-26 BlackThrush
  — typed_float_unary generic+move: … floor/ceil/trunc/round/sqrt are a SIMD
  BUILD-TARGET blocker"*, six recorded losses vs pandas:
  **floor 0.089x, ceil 0.11x, trunc 0.13x, round(decimals) 0.090x, sqrt ~0.085x,
  log 0.20x.**

## Retry predicate: SATISFIED, verified on hardware

The row's own predicate is *"the ceiling for the math-unary family **until that
build-target call is revisited**."* `fbc5b3f60` asserted this was satisfied because
`ovh-b` lost its `rust` tag. That was taken as a claim to check, not a fact:

Every rust-tagged worker was probed individually — **11/11 carry avx2 + bmi2 +
fma**, so the Rust fleet's ISA floor genuinely is x86-64-v3. `workers.toml`
confirms the single exception is excluded by standing policy, not luck: `ovh-b` is
`["bun","go","no-avx2"]` with *"2026-07-25 (orchestrator, fleet-wide
confirmation): DO NOT ADD `rust` BACK."*

## Mechanism: confirmed on a HEAD-current binary pair

Two `fp-bench` ELFs, both from **clean `HEAD`** via
`rch exec --base HEAD --clean-overlay --no-overlay`, differing only in
`RUSTFLAGS="-C target-cpu=x86-64-v3"`:

| | baseline | x86-64-v3 |
|---|---|---|
| sha256 | `e231b31837875ba599db08148b2499afcad4c792ddc6283c6038dee832a46d86` | `ada5eab8584404622667276f614da0ee8b27e4e59315cc31e1e04816a8953d80` |
| bytes | 75 019 272 | 75 329 840 |
| built on | `vmi1152480` | `vmi1153651` |

`objdump -d` instruction census:

| instruction | baseline | v3 | note |
|---|---:|---:|---|
| `vroundpd` | **0** | **17** | the instruction the ledger says is missing |
| `vroundsd` | 0 | 91 | |
| `vsqrtpd` | **0** | **7** | |
| `vsqrtsd` | 0 | 66 | |
| `vmulpd` | 0 | 71 | |
| `vaddpd` | 0 | 149 | |
| `vandpd` | 0 | 117 | |
| legacy `mulpd` | 115 | **0** | SSE forms fully replaced |
| legacy `addpd` | 95 | **0** | |
| legacy `sqrtsd` | 64 | **0** | |
| legacy `sqrtpd` | 4 | **0** | |

The ledger's stated blocker — *"these need `vroundpd` (SSE4.1) / `vsqrtpd`-wide …
but fp builds for GENERIC x86-64"* — is resolved at the instruction level on a
HEAD-current build. This reproduces `fbc5b3f60`'s finding (which reported
vroundpd 12 / vroundsd 80 / vsqrtpd 5 / vsqrtsd 57) with the small differences
expected from a tree that has moved since.

### A wrong turn worth recording

I first inspected a `fp-bench` on `vmi1227854`, found a pure baseline instruction
mix, and was about to conclude that `RUSTFLAGS` does not propagate through
`rch exec -- env RUSTFLAGS=… cargo build`. **That was wrong.** My build had
actually landed on `vmi1153651` (`[RCH] remote vmi1153651 (1127.8s)`); the
`vmi1227854` binary was a *peer's* concurrent fp-bench build. The pool-hash
suffix is identical across workers (`…-pool-bc445989bdf88102bcbc62abd4347d69`), so
the path alone does not identify whose build it is. **Always take the worker
identity from the rch output, never from finding a plausible binary.** RUSTFLAGS
propagates fine.

## What is still missing: the timed A/B

Per the fleet rule *"never gate an ISA change on instruction count; fewer
instructions is the mechanism, not a neutral proxy"*, this row needs a
**whole-binary timed A/B against live pandas** before any verdict changes. That
did not run. The blocker is documented in
`cc_blackthrush_same_invocation_blocker_20260731.md`: the harness's quiescence
preflight samples immediately after the harness itself hashes 228 MB of
pandas+pyarrow, so on a 10-core worker it fails closed on its own startup — shown
by launching only from a verified sub-5%-on-every-core state and still getting
`busy=[0..9]`.

**The row therefore stays as it is. No verdict is changed, and no ratio is
claimed.** Its status moves from *"blocked on a build-target decision nobody has
made"* to *"blocked only on instrument time"*, which is a real move and the reason
this file exists.

## Staged so the re-test is a pure "run it"

Both ELFs are on `vmi1149989` and self-report their own SHA-256 from inside the
measured process:

```
/root/fpbench-bin/fp-bench      e231b318…  bench_elf_sha256 verified in-process
/root/fpbench-bin/fp-bench-v3   ada5eab8…  bench_elf_sha256 verified in-process
```

pandas 2.2.3 + pyarrow 24.0.0 live at `/root/fpbench-libs`. `--category
math_unary` covers exactly the six ledger ops (floor, ceil, trunc, round2, sqrt,
log). Once the preflight is reordered, the command is:

```
python3 benches/vs_pandas_harness.py --category math_unary --sizes 1M \
  --workloads <one op> --thread-count 10 \
  --expected-hostname vmi1149989 --expected-physical-cores 10 \
  --expected-logical-threads 10 \
  --frankenpandas-binary /root/fpbench-bin/fp-bench{,-v3}
```

one op per invocation (multi-workload runs die on prior-arm residue).
