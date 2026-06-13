# br-frankenpandas-a3y5o: contiguous UTF-8 factorize byte-span rejection

## Target

- Bead: `br-frankenpandas-a3y5o`
- Selection path: `br ready --json` had no unclaimed ready `[perf]` beads; the
  live perf beads were already claimed. The fallback target was chosen from the
  current `perf_profile` harness.
- Profile-backed baseline matrix:
  `tests/artifacts/perf/vcstr_baseline_cardinality_matrix_500k.json`
- Hottest measured contiguous-UTF8 cardinality scenario:
  `str_factorize 500000 5` at `93.8 ms +/- 3.7 ms`.

## Baseline

Built through RCH:

```text
CARGO_TARGET_DIR=.rch-target-vcstr-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Focused baseline:

```text
str_factorize 500000 5: 85.9 ms +/- 4.1 ms
```

Goldens:

```text
str_factorize 1000:   cd726ef905573f7e869818443fb3320d49ecf7fbc0556ca2d2d6794743491fc4
str_factorize 500000: da2da85584bdca84af3055c9104c531d5bdf760b191420e8aa5f8366cfd3fe11
```

`perf stat` was unavailable:

```text
perf_event_paranoid setting is 4
```

## Rejected Lever

Candidate: for `factorize(sort=false, use_na_sentinel=true)` over all-valid
contiguous UTF-8, assign first-seen codes from borrowed byte spans and build the
unique UTF-8 byte buffer directly, instead of materializing every source row as
`Scalar::Utf8`.

Behavior stayed identical:

```text
cmp_1000=0
cmp_500000=0
```

After golden hashes matched the baseline hashes exactly.

## Bench Gate

Paired `500000x5` was noisy and not accepted:

```text
forward:  baseline 105.5 ms +/- 4.8 -> candidate 86.4 ms +/- 3.2 (candidate 1.22x)
reversed: candidate 85.7 ms +/- 2.5 -> baseline 84.8 ms +/- 4.1 (neutral)
```

Higher-iteration gate rejected the lever:

```text
forward 500000x50:  baseline 372.3 ms +/- 13.6 -> candidate 398.3 ms +/- 28.5
reversed 500000x50: candidate 381.1 ms +/- 28.7 -> baseline 367.2 ms +/- 18.7
```

Score: `< 2.0`; source hunk removed.

## Isomorphism Notes

- Ordering: candidate assigned codes in row order and appended each unique byte
  span on first encounter, matching existing first-seen factorize semantics.
- Tie-breaking: no sorting was introduced; duplicate keys used byte equality
  against the same UTF-8 bytes as `Scalar::Utf8(s).as_str()`.
- Missing values: the route was gated to all-valid contiguous UTF-8 with
  `use_na_sentinel=true`; nullable and non-default semantics stayed on the
  existing path.
- Floating point/RNG: no floating-point arithmetic or random behavior was
  touched.

## Rejection Diagnosis

The harness repeats factorize on one `Series`. The current implementation pays
the lazy contiguous-UTF8 scalar materialization cost once, then reuses the
cached scalar view for later iterations. The byte-span candidate avoided the
first materialization but rehashed borrowed spans on every iteration, so it lost
on the steady-state `x50` gate.

Next primitive: cache a dictionary/factorization witness on the contiguous UTF-8
backing, or build a reusable byte-span dictionary representation shared by
`factorize`, `unique`, `duplicated`, and `value_counts`. Target ratio: at least
`1.25x` on `str_factorize 500000x50` and neutral on `str_unique`/`str_duplicated`.
