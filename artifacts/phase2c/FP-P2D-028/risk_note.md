# FP-P2D-028 Risk Note

Primary risk: axis=1 alignment drift can silently misplace row values across columns, causing hard-to-detect downstream parity regressions.

Mitigations in this packet:

1. Fixture matrix pins deterministic union-index ordering (`left` then unseen from `right`).
2. Null-fill behavior is verified across sparse and non-monotonic index alignments.
3. Fail-closed diagnostics are enforced for unsupported axis selectors, duplicate columns, and duplicate index labels.

Invariant hooks:

- `FP-I1` (shape consistency): output row count equals deterministic union index cardinality.
- `FP-I4` (determinism): repeated axis=1 concat yields stable index/value ordering.
- `FP-I7` (fail-closed semantics): unsupported or ambiguous selector surfaces are explicit errors.

Residual risk:

- Duplicate-column preservation and MultiIndex concat semantics remain out of scope for this packet and must be covered in a follow-up matrix.

