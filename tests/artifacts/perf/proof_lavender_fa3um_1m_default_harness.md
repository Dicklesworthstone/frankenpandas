# br-frankenpandas-fa3um proof: default vs-pandas harness includes 1M

## Lever

Change the vs-pandas harness default size frontier from `10k,100k` to
`10k,100k,1M`, and update the benchmark matrix join rows so left/inner/outer
join routing is also assessed at 1M scale.

This is a benchmark-routing lever, not a FrankenPandas runtime optimization. It
does not change library execution, dataframe semantics, ordering, RNG, or
floating-point behavior. Its purpose is to keep fixed per-operation overhead at
100k from being misclassified as an algorithmic vs-pandas performance gap.

## Before/after behavior

Before artifact:

```text
tests/artifacts/perf/lavender_fa3um_before_default_sizes.txt
```

Relevant baseline lines:

```text
python benches/vs_pandas_harness.py --all --sizes 10k,100k
parser.add_argument("--sizes", default="10k,100k",
```

After artifact:

```text
tests/artifacts/perf/lavender_fa3um_after_default_sizes.txt
```

Relevant candidate lines:

```text
python benches/vs_pandas_harness.py --all --sizes 10k,100k,1M
parser.add_argument("--sizes", default="10k,100k,1M",
```

CLI help artifact:

```text
tests/artifacts/perf/lavender_fa3um_after_help_sizes.txt
```

Relevant line:

```text
--sizes SIZES         Comma-separated sizes (10k,100k,1M)
```

## Hashes

```text
d9c4b231b0157b501f0d7abd564b7058b6139dbb7f5fd04d4ce417e28d8ec484  tests/artifacts/perf/lavender_fa3um_before_default_sizes.txt
37a11085666c4126ea964d1acd98808ecd550284403c44cc9f2b4a22ddee23ca  tests/artifacts/perf/lavender_fa3um_after_default_sizes.txt
a3606a492330138d7b5bb9b8bffebbbbf8c6422ac086a48129cc8ae27cf26ee0  tests/artifacts/perf/lavender_fa3um_after_help_sizes.txt
```

## Validation

```text
python3 -m py_compile benches/vs_pandas_harness.py
PASS

python3 benches/vs_pandas_harness.py --help | rg -n -- '--sizes|10k,100k,1M'
PASS
```

## Isomorphism

- Library behavior is unchanged: no Rust source, dataframe operation, join
  algorithm, output order, dtype coercion, null handling, RNG, or floating-point
  arithmetic path changed.
- Explicit `--sizes` callers keep exact behavior. Only the omitted-argument
  default changes.
- The harness already had a `SIZE_CONFIGS["1M"]` entry; this change makes the
  existing supported size part of the default frontier.
- The spec documentation now matches the measurement doctrine used by the
  profile-backed beads: 1M scale is the routing frontier for fixed-overhead
  discrimination.

## Score

```text
Impact 4 x Confidence 1.00 / Effort 1 = 4.00
Decision: keep
```
