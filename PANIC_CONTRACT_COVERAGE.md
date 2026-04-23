# Panic-Contract Coverage Tracker

Per br-frankenpandas-z9a8 (follow-up to br-frankenpandas-7cfm).

**Policy.** Every public function whose implementation contains a panic
path (`.unwrap()`, `.expect(...)`, `unreachable!()`, `panic!()`, or an
out-of-bounds indexing pattern that the compiler cannot prove safe) must
document the trigger conditions under a `# Panics:` section in its
rustdoc. Panics are part of the API contract; readers shouldn't have to
grep source to learn when a function will crash their process.

## Coverage baseline (2026-04-23)

| Crate           | `pub fn` count | `# Panics:` blocks | Coverage |
| --------------- | -------------: | -----------------: | -------: |
| fp-frame        |         (many) |             5      | *(baseline wave — br-7cfm)* |
| fp-io           |            49  |             0      |     0.0% |
| fp-expr         |            10  |             0      |     0.0% |
| fp-groupby      |            18  |             0      |     0.0% |
| fp-join         |             9  |             0      |     0.0% |
| fp-index        |         (many) |             0      |     0.0% |
| fp-columnar     |         (many) |             0      |     0.0% |

## Action plan

Priority order (per bead description): fp-io → fp-expr → fp-groupby →
fp-join → remaining crates.

Per-crate sweep methodology:
1. `grep -n 'unwrap\|expect\|unreachable\|panic!' crates/<name>/src/`
2. For each `pub fn` whose body contains one of those, add a `# Panics:`
   rustdoc block documenting the trigger. If the panic is unreachable
   in practice (e.g. invariant enforced upstream), note that.
3. Convert obviously-infallible panics (e.g. `.unwrap()` on a just-
   created `Result<_, Infallible>`) to `.unwrap_or_else` or expect-with-
   semantic-message while documenting the invariant.
4. Rustdoc warning tighten once per-crate coverage ≥ 95%: add
   `#![warn(clippy::missing_panics_doc)]` to that crate's lib.rs.

## Related beads

- br-frankenpandas-7cfm (closed): fp-frame first-wave sweep + rustdoc
  CI gate.
- br-frankenpandas-ddox (closed): doctest CI step ensures documented
  examples stay runnable; doctests provide the complementary "how to
  call this without panicking" half.
- br-frankenpandas-niwb: error-path conformance overlaps in spirit —
  a fn that returns `Err` instead of panicking is often the right
  refactor for a poorly-documented panic.
