# br-frankenpandas-jawxr - FMA/golden-regeneration decision

Timestamp: 2026-06-13T11:27:00Z
Agent: LavenderStone

## Target

`br-frankenpandas-jawxr` asks for orchestrator sign-off to regenerate
`corr`/`cov`/`spearman`/`kendall` numerical goldens around `f64::mul_add` or a
workspace FMA contract. The bead's own description states the key risk:
`f64::mul_add` is safe Rust and can be faster, but it changes the floating-point
result bits through single-rounding and reassociation.

## Active Contract

The active campaign directive requires:

- one optimization lever per commit,
- behavior unchanged,
- ordering/tie-breaking/FP-bit/RNG isomorphism proof,
- golden-output sha256 verification,
- keep only Score >= 2.0.

The no-gaps directive also states that behavior parity is absolute: a faster
kernel that changes a return value, tie-break, FP bit pattern, or error class
does not ship.

## Decision

Reject the requested FMA/golden-regeneration approval in this optimization
pass. This is not a safe optimization lever under the active contract because
the proposed acceptance criterion explicitly changes FP bits and requires new
goldens instead of verifying the current golden SHA.

This is not a statement that the numerical gaps are exhausted. It only rejects
this behavior-changing contract. Future valid work must use a profile-backed
target whose proof can preserve the current observable contract, or it must be
carried under a separate explicit behavior-contract change outside this
optimization loop.

## Evidence

Existing profile/commit trail:

- `77157629` kept the bit-compatible parallel `df.corr()`/`df.cov()` complete
  Gram and moments path for `br-frankenpandas-jawxr`.
- `5e4970aa` kept the communication-avoiding row-partitioned Gram for
  `br-frankenpandas-fbav3`.
- `9be363ee` kept the Spearman centered-rank Gram route for
  `br-frankenpandas-vhrv5`.
- Current code comments in `pairwise_stat_matrix` explicitly document that
  reassociated Gram folds are a contract-sensitive numerical choice.

## Route

Do not spend more optimization passes on FMA unless the human explicitly changes
the behavior contract. Continue with exact, profile-backed non-FMA work:

- true cross-column Kendall dominance sharing,
- typed-columnar producer/consumer boundaries that keep existing goldens,
- exact join/filter materialization primitives,
- safe SIMD/string/hash/table kernels where byte-for-byte output is preserved.

Non-repeat boundary: no FMA, `mul_add`, golden-regeneration, or workspace
floating-point contract changes in the strict optimization loop.
