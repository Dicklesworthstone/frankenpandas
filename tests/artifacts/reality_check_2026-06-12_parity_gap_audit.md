# Reality-check / parity-gap audit — 2026-06-12 (BlackThrush)

Phase-B generative pass after the autonomous perf surface was exhausted to
memory/compute bounds (see the recent rejection witnesses: dwm2u str-dedup,
0dm7c kendall, plus the reverted spearman-ranks radix experiment this session).
This is a deliberate audit of where FrankenPandas still diverges from / lags
pandas, with concrete beads filed for each actionable gap.

## Perf state (why no autonomous ≥2× win is available right now)
- **corr/cov/spearman Gram (~900ms, jawxr/bh7iy/vhrv5):** bit-identical Gram is
  serial-add / FP-latency bound (~3 GFLOP/s); reassociation and multi-accumulator
  ILP both break the golden. The only ≥2× lever is FMA (`f64::mul_add`), which
  shifts bits → needs the **jawxr golden-regen sign-off (orchestrator decision)**.
  This is the single highest-leverage perf unblock and is NOT autonomous.
- **kendall (df_kendall ~617ms, 0dm7c):** the u32 Fenwick is L3-resident, so it's
  compute-bound, not cache-bound; merge-sort and √-decomposition both regress
  (proven). No data-structure swap beats it.
- **spearman ranking (`average_ranks`):** radix argsort REGRESSES vs the in-place
  comparison sort (per-column key/perm allocations + random-access tie walk).
- **joins / sort_multi / columnar / index / groupby:** typed gathers, radix
  sorts (skip constant byte-passes), FxHash build/probe — all already optimized.

## Parity gaps found (beads filed)
- **br-frankenpandas-i10en** — `set_index` rejects Float64/Bool columns;
  `IndexLabel` lacks those variants (Float64 needs a bit-pattern total-order/Hash
  wrapper). Cross-cutting; Float64Index is the common case.
- **br-frankenpandas-rj4fn** — `asfreq`/resample reject W/Q/B frequencies;
  `asfreq_target_labels` is start-anchored stepping, while pandas anchors W→W-SUN,
  Q→Q-DEC, M→month-end. Needs the anchoring conventions validated vs the pinned
  oracle (so NOT a safe blind add — parity is absolute).
- **br-frankenpandas-krw0g** — `to_period`/`to_timestamp` reject several Period
  frequencies ("not supported yet").
- **br-frankenpandas-0ezw7** (pre-existing) — `to_datetime`/CSV produce
  `Scalar::Utf8` (re-parsed per dt access) where pandas returns datetime64.
  CONFIRMED multi-session: `parse_datetime_string` has ~14 Utf8 return sites incl.
  a "loose passthrough" that returns unparsed-but-datetime-looking strings (can't
  become typed → mixed-dtype columns), plus tz-aware handling and an exact
  `format_naive_datetime` vs `format_datetime_ns` round-trip requirement.

## Already-closed this run
- **br-frankenpandas-261** — clipboard I/O implemented via zero-dep OS subprocess
  backend (commit 6141f282), advancing the absolute-parity mandate over the prior
  "headless charter" deferral.

## Recommendation
The biggest single win remaining is the **jawxr FMA build-decision** (~900ms,
orchestrator sign-off). The datetime parse-once (0ezw7) is the biggest autonomous
architectural swing but genuinely multi-session. The three new parity beads are
bounded but each needs oracle validation (frequencies/anchoring) or a
cross-cutting `IndexLabel` change — none is a clean one-hour, parity-safe, blind
implementation.
