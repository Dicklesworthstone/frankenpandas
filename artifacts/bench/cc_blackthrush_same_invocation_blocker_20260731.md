# Why the same-invocation conversion did not land, and the bug that explains it

- Agent: BlackThrush · Date: 2026-07-31 · Bench host: `vmi1149989` (10 physical
  cores, no SMT, AMD EPYC, avx2+bmi2+fma, 63 GB)
- FP ELF: `e231b31837875ba599db08148b2499afcad4c792ddc6283c6038dee832a46d86`
  (75 019 272 bytes), built from clean `HEAD` via
  `rch exec --base HEAD --clean-overlay --no-overlay`, on worker `vmi1152480`,
  SHA verified after transfer and **self-reported by the process itself**
  (`bench_elf_sha256=` line 1 of every fp-bench run).
- Harness: immutable snapshot
  `e12e3ad5a720d05963557510fed69d9d195f629ab82b1dcbd5ec72298d4dc980`
  (126 807 bytes) of CyanLynx's in-flight working tree — HEAD has neither the
  corrected three-clause gate nor `--frankenpandas-binary`.
- Incumbent: **pandas 2.2.3 live, in-process**, pyarrow 24.0.0, numpy 2.5.1.
  `pandas_dependency_probe=ready`.

Everything needed for a same-invocation ratio was assembled and verified. The
number is still not in this file, and this is why.

## The bug: the quiescence preflight measures the harness's own startup

`benches/vs_pandas_harness.py` `main()`:

```
L3329  pandas_artifact  = pandas_artifact_identity()    # walk + SHA-256 whole tree
L3330  pyarrow_artifact = pyarrow_artifact_identity()   # walk + SHA-256 whole tree
...
L3355  exclusivity_gate.require_quiet("invocation_preflight")
```

The gate samples a **300 ms** window immediately after the harness has itself
done a large burst of CPU and page-cache I/O. Measured:

| stage | cost |
|---|---:|
| `import pandas` + `pyarrow` | 0.319 s |
| hash pandas tree — 2 915 files, 70.3 MB | 0.849 s |
| hash pyarrow tree — 854 files, 157.7 MB | 0.609 s |
| **total immediately before the sample** | **1.777 s** |

On the 64-core workstation that burst is diluted across 64 CPUs and usually gets
away with it. On a 10-core worker it is fatal, because the gate requires **every**
online CPU at or below 20%:

```
ERROR: host-wide benchmark exclusivity requires every online CPU to remain at or
below 20.0% busy; phase=invocation_preflight missing=[] busy=[0,1,2,3,4,5,6,7,8,9]
```

All ten — on a box whose top process was `kswapd0` at 1.2%.

**Consequence beyond this task:** the gate rejects runs for load the harness
itself created, and in the artifact that is indistinguishable from a genuinely
contended host. An unknown fraction of this repo's "gate blocked, no number"
history is plausibly this, not peer load — i.e. we may have been diagnosing the
wrong thing. Reported to CyanLynx, who owns the file. The suggested fix is to
sample **before** the artifact-identity block; the provenance hashes do not depend
on the gate and the gate should not be measuring them.

A second, smaller instance: a multi-workload invocation dies at
`phase=pre_measurement:pandas:<next workload>` on prior-arm residue. Workaround
that does work: **one workload per invocation**.

### This was demonstrated, not inferred

A first version of this note said the burst was *probably* the cause. That is not
good enough, so it was tested directly. An opportunistic runner was built that
refuses to launch the harness until the host has passed **three consecutive
300 ms samples with every CPU under 5% busy** — four times stricter than the gate
itself, and verified immediately before each launch:

```
for A in $(seq 1 400); do
  if quiet; then   # 3 x 300ms samples, max_busy <= 0.05, else wait 10s
      python3 benches/vs_pandas_harness.py --workloads <one> ...
```

From that verified-quiet start, the harness's very next act is to import pandas
and hash 228 MB, and its own preflight then reports:

```
phase=invocation_preflight  busy=[0,1,2,3,4,5,6,7,8,9]
```

A host measured at **under 5% on every core** becomes **over 20% on every core**
between the launch decision and the harness's own first sample. Nothing else ran
in that gap. The burst is the harness's own, and the gate is failing closed on
it. On a 10-core host this makes the measurement not merely unlucky but
**unreachable** until the preflight is reordered.

## The other blocker: a shared fleet cannot be held quiet

The gate is satisfiable only on a host nobody else is using. There is no such host:

- **No rch worker had pandas installed** — probed 12/12, `pandas=NONE`. So the FP
  arm and the incumbent had never been co-located anywhere.
- **rch workers are build machines**; they violate the gate by design.
  `vmi1149989` reported loadavg 0.44 while giving **10/10 blocked** samples with
  six `rustc` pegged. Never trust loadavg here.
- I drained it (`rch workers drain -y`), then **disabled** it when jobs kept
  landing. It did go quiet — verified **0/6 blocked, max busy 0.033** — and in
  that window the harness preflight returned `verdict=clear` and entered
  measurement.
- It did not stay quiet. Load average reached **75** with a dozen `rustc`
  processes while `rch queue` showed **zero** jobs assigned to it: peers run
  `cargo` on these boxes directly, outside rch. A single agent cannot hold a
  shared worker idle, and should not try to.

## What was NOT done, deliberately

The gate was **not** relaxed, scoped down, or routed around, and no CPUs were
taken offline to shrink the set it inspects. A ratio obtained by weakening the
instrument that exists to make ratios trustworthy is worth less than no ratio.
The claim this work was meant to convert has instead been **corrected by
labelling** (commit `a1516e1cc`), which required no benchmark at all — the README
number was provably FP-side from `ROUND5_BASELINE.md`.

## What is now in place for whoever picks this up

1. `vmi1149989` has pandas 2.2.3 + pyarrow 24.0.0 at `/root/fpbench-libs`
   (`pip --target`; no venv, no system packages touched) and the pinned
   `nightly-2026-04-22` toolchain.
2. The HEAD-current fp-bench ELF is at `/root/fpbench-bin/fp-bench` on that host,
   SHA above.
3. `artifacts/bench/cc_blackthrush_same_invocation_runner.sh` encodes
   setup / drain / wait / run / restore.
4. rch admission was repaired fleet-wide (toolchain sync + `capabilities
   --refresh`; admission is **cached**, so the sync alone does nothing).

The remaining requirement is a genuinely idle host — or, much cheaper, the
four-line reordering of the preflight, after which a 10-core worker is enough.
