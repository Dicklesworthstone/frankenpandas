# br-frankenpandas-uza04.133 df_dot BJ=8 rejection proof

Agent: LavenderStone
Date: 2026-06-14

## Target

`df_dot 100000x5` remained the largest measured residual after
`br-frankenpandas-uza04.132`:

- `df_dot 100000x5`: ~145.900 ms/iter in the post-closeout routing matrix.

Prior `br-frankenpandas-vc4de` rejected metadata/chunk-handoff routes, so this
pass tested a deeper layout primitive: widen the packed B panel from 4 to 8
columns while keeping `BI=4`.

## Candidate Lever

Changed only the dot micro-kernel panel shape:

- Before: `BI=4`, `BJ=4`
- Candidate: `BI=4`, `BJ=8`

Isomorphism argument:

- Output labels and column order are assembled by the same post-kernel path.
- Null/NaN behavior is unchanged because all output columns use the same
  `column_has_nan` witness and `Column::from_f64_all_valid_chunks` /
  `Column::from_f64_values` branch.
- Each cell keeps the same depth loop order, `l = 0..k`, with the same
  `acc += a_cols[l][row] * b[j][l]` products. Widening the independent `j`
  panel does not reassociate any one cell's sum.

## Golden Proof

Baseline:

- `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
- `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

Candidate:

- `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
- `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

Goldens are byte-identical by sha256.

## Timing

Build path used the RCH wrapper, but RCH fell open to local for both baseline
and candidate because no admissible worker was available.

Internal timing:

- Baseline: `129.501 ms/iter`
- Candidate: `197.056 ms/iter`

Hyperfine, 7 runs, command `/data/tmp/cargo-target/release-perf/examples/perf_profile df_dot 100000 5`:

- Baseline: `691.2 ms +/- 9.0 ms`
- Candidate: `1.038 s +/- 0.040 s`

Result: rejected. The wider panel increased register pressure / scheduling cost
and regressed by roughly 50%.

Source hunk removed before commit. Do not retry simple `BJ` widening; the next
df_dot pass needs a different algorithm/layout primitive, not panel-width tuning.
