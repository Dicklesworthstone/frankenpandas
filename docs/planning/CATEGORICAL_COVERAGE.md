# Categorical Dtype Conformance Tracker

Per br-frankenpandas-2ssw.

README claims Categorical is a "metadata layer with pandas parity." The
single fixture for Categorical today (1 of 1,249) does not prove this
claim for the ~50+ methods pandas exposes on Categorical Series.

## Method coverage (baseline — 2026-04-23)

Pandas' public Categorical surface, grouped by the phases this tracker
sequences:

**Phase 1 — Constructors (0 / 10 covered):**
`Categorical(values)`, `Categorical(values, categories=...)`,
`Categorical(values, ordered=True)`,
`Categorical.from_codes(codes, categories, ordered)`,
`Series(values, dtype="category")`, `Series.astype("category")`,
`pd.CategoricalDtype(categories, ordered)`,
`CategoricalIndex(values)`, `Categorical(nan_values, ...)`,
`Categorical(ordered=True, different_category_order)`.

**Phase 2 — Mutation (0 / 10 covered):**
`add_categories`, `remove_categories`, `remove_unused_categories`,
`rename_categories`, `reorder_categories`, `set_categories`,
`as_ordered`, `as_unordered`, `fillna`, `map`.

**Phase 3 — Comparison + equality (0 / 10 covered):**
`==`, `!=`, `<`, `>`, `<=`, `>=` across ordered/unordered,
`equals` across category-order differences,
`Categorical.compare` mixed-type cases.

**Phase 4 — Sort (0 / 10 covered):**
`sort_values` ordered=True respects category order,
`sort_values` ordered=False falls back to lex,
`argsort`, `rank`, `nsmallest`, `nlargest`,
MultiIndex sorts involving Categorical levels.

**Phase 5 — GroupBy interaction (0 / 10 covered):**
`groupby(observed=True)` drops unused (pandas 2.1+ default),
`groupby(observed=False)` keeps empty groups,
`groupby().agg(...)` per agg-name,
`groupby().transform(...)` preserves categories.

**Phase 6 — Concat / union / reshape (0 / 10 covered):**
`pd.concat` dtype-consistent categories coerce,
`pd.concat` mismatched categories raise or coerce,
`Series.append`, `DataFrame.join` column-Categorical,
`Series.unstack`, `Categorical.describe`.

**Phase 7 — Error paths (0 / 10 covered):**
TypeError on arithmetic, ValueError on unknown category,
TypeError on ordered ops against unordered, etc.
(Ties to br-frankenpandas-niwb error-conformance tracker.)

Target: lift coverage to 70% of pandas' Categorical surface over the
seven phases.

## Oracle handlers needed

`crates/fp-conformance/oracle/pandas_oracle.py` today exposes only
`op_series_from_codes` (per br-2ssw bead description). Phase 1
additions will land `op_categorical_{constructor variants}`; each
subsequent phase extends the dispatch alongside its fixtures.

Per the br-urhy oracle-pytest tracker, every new op handler must arrive
with a matching unit test under `oracle/tests/`.

## Related beads

- br-frankenpandas-urhy (closed): oracle pytest suite. Phase-1+ op
  handlers ship their tests into that suite.
- br-frankenpandas-niwb: error-conformance tracker. Phase 7 fixtures
  overlap.
- br-frankenpandas-nato: RequirementLevel tagging. Categorical
  fixtures at Must vs Should level drive compliance-score deltas.
