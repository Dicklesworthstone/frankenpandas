# Differential-Fuzz Architecture (br-frankenpandas-liai)

Today all 24 fuzz targets use crash-only or invariant oracles. Per
`/testing-fuzzing` Oracle Hierarchy, a reference implementation is
the strongest oracle available — pandas is that reference, and
`pandas_oracle.py` already answers arbitrary op payloads for the
conformance harness. This doc pins the architecture for wiring a
differential fuzz target on top of that existing channel.

## Target shape

```rust
// crates/fp-conformance/src/lib.rs — future slice
pub fn fuzz_differential_vs_pandas_bytes(input: &[u8]) -> Result<(), String> {
    let op = pick_op_from_byte(input);                  // byte 0
    let frame = fuzz_eval_frame_from_bytes(&input[1..])
        .map_err(|e| format!("frame projection failed: {e}"))?;

    let rust_result = dispatch_rust_op(op, &frame)?;
    let pandas_result = call_pandas_oracle(op, &frame)?;

    if !semantic_eq(&rust_result, &pandas_result) {
        return Err(format!(
            "differential divergence: op={op:?} rust={rust_result:?} pandas={pandas_result:?}"
        ));
    }
    Ok(())
}
```

## Op alphabet (first wave)

Map one byte tag to one op-string the oracle already dispatches:

| Tag | Op string           | Rust entry point                     |
| --: | ------------------- | ------------------------------------ |
|   0 | `nan_sum`           | `nansum(&column)`                    |
|   1 | `nan_mean`          | `nanmean(&column)`                   |
|   2 | `nan_min` / `max`   | `nanmin` / `nanmax`                  |
|   3 | `series_add`        | `Series::add`                        |
|   4 | `groupby_sum`       | `DataFrame::groupby(k).sum()`        |

Expanding the alphabet is mechanical once the scaffolding is in place.

## Subprocess overhead

`pandas_oracle.py` startup cost is ~80-120 ms; steady-state RPC per
payload is ~20-40 ms. Fuzz exec/s floor per skill rule 1:

- Parsers (single-threaded): 1000 exec/s → differential target would
  miss that floor by two orders of magnitude.
- Stateful targets: 100 exec/s → still miss by ~5x.
- Long-lived oracle process with framed stdin/stdout: ~250 req/s
  (measured on similar integrations). Approach the 100 floor if we
  amortize startup across many payloads.

**Decision:** ship under a dedicated cargo-fuzz target that runs in
the nightly window only, not the PR-gate corpus. `fuzz_parallel_data
frame` sets the precedent (weekly TSan cadence).

## Long-lived oracle protocol

Minimum viable scaffold to unblock the slice:

```python
# pandas_oracle.py already has a --batch flag pattern? Verify.
# If not, add a loop:
#   while line := sys.stdin.readline():
#       request = json.loads(line)
#       response = dispatch(pd, request)
#       sys.stdout.write(json.dumps(response) + "\n")
#       sys.stdout.flush()
```

Bonus: the same protocol unlocks benchmark-mode pandas comparisons
(br-xha7 Phase 2).

## DISCREPANCIES.md interaction

Every divergence the harness reports falls into one of three buckets:

1. **Bug** — we're wrong, pandas is right; file a fix bead.
2. **Bug in pandas** — edge case where our behavior is closer to the
   documented contract; file a DISCREPANCIES.md entry explaining the
   preferred semantics.
3. **Known-intentional divergence** — already in DISCREPANCIES.md;
   the harness must skip or XFAIL.

The comparator takes a `DISCREPANCIES.md` allow-list (parsed from the
`Tests affected:` lines); hits on that list are not reported as
regressions.

## Implementation roadmap

- **Phase 1 (now):** this design doc + tracker.
- **Phase 2:** long-lived oracle subprocess + minimal alphabet (5
  ops) + in-harness comparator.
- **Phase 3:** nightly CI workflow running the target for 60 min.
- **Phase 4:** expand op alphabet to match the conformance suite's
  430 covered ops.

## Related beads

- br-urhy (closed): pandas_oracle.py pytest suite. Fuzz divergences
  should cross-check against the oracle's own unit tests first.
- br-xha7 (closed): differential criterion bench scaffold. Shares the
  long-lived-oracle pattern this doc specifies.
- br-kdwn (closed): reverse-conformance oracle. Complementary channel
  (pandas reads our output); this bead is forward-differential.
