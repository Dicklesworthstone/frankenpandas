# fp-expr

Expression evaluation / query parser for fp-frame DataFrames —
pandas `df.eval` / `df.query` parity.

Part of the [frankenpandas](https://github.com/Dicklesworthstone/frankenpandas)
workspace.

## Public API

- `eval_str(expr, df, policy, ledger)` — pandas `df.eval(expr)`
  parity. Returns a Series.
- `query_str(expr, df, policy, ledger)` — pandas `df.query(expr)`
  parity. Returns a filtered DataFrame.
- `parse_expr(s)` — AST-level parser. The lowest-level surface;
  useful when you want to inspect or transform an expression
  before evaluating.
- `_with_locals` variants — accept a `BTreeMap<String, Scalar>` of
  named locals for expression substitution.

## When to depend on fp-expr directly

Most users access these through `frankenpandas::DataFrame::eval`
and `DataFrame::query` methods (the umbrella crate re-exports
them). Direct dependency makes sense when:

- You want to pre-parse an expression once and evaluate many
  times across different frames.
- You want custom policy / ledger handling around eval calls.

## Status

Stable. `ExprError` is `#[non_exhaustive]` per br-frankenpandas-tne4.
Fuzz coverage: `fuzz_dataframe_eval` target; `parse_expr` /
`query_str` fuzz targets tracked under br-frankenpandas-jkhg.

## Links

- [Workspace README](../../README.md)
- [CHANGELOG](../../CHANGELOG.md)
