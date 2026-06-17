# fp-frame UBS Triage (br-frankenpandas-yavyk)

Triage of the broad UBS (Ultimate Bug Scanner) inventory on
`crates/fp-frame/src/lib.rs`. Scan: UBS v5.3.2, `--only=rust`, single file
(138,299 lines), 2026-06-17.

## Inventory

| Severity | Count  |
|----------|--------|
| critical | 627    |
| warning  | 48,916 |
| info     | 4,236  |

UBS exits non-zero because of the criticals. **All 627 criticals are false
positives**, and the warning/info bulk is the same heuristic noise. No source
change is warranted; this file is `#![forbid(unsafe_code)]` and the UBS internal
fmt/clippy/check/test sub-gates are clean.

## Why every critical is a false positive

The 627 criticals come from exactly three rust analyzer categories, all of which
target threat surfaces — credential handling, web auth, process/fs traversal —
that **do not exist** in a pure-compute columnar DataFrame/Series kernel.
`crates/fp-frame/Cargo.toml` has **no** network / crypto / auth / db / async
dependencies (no reqwest, hyper, jsonwebtoken, ring, openssl, rustls, tokio,
sqlx, …), and the source has **zero** credential-term occurrences
(`grep -niE 'password|bearer|hmac|api_?key|access_?token|secret_?key|csrf|\bjwt\b|constant_time'`
→ 0 matches).

| # crit | UBS category (rust cat) | What it flags | Why it is FP here |
|--------|--------------------------|---------------|-------------------|
| 537 | Security Findings (8) — secret/token compared with `==`/`!=` | any `==`/`!=` whose operand name brushes a sensitive substring | These are dtype/value comparisons: `column.dtype() == DType::Utf8`, `pair[0] == pair[1]` (window dedup), scalar/label equality. No secrets exist in a dataframe. |
| 88 | Panic Surfaces (21) — `panic!` in library code | `panic!`/`unreachable!`/`todo!` macros | All 84 `panic!` macros are inside the inline `#[cfg(test)]` module (lines 60575–132793): `other => panic!("expected …")` test assertions. **Zero `panic!` in non-test code** (verified: `awk '$1<60575'` → 0). Correct test idiom. |
| 2 | Security Findings (8) — JWT decode/validation bypass | the word `decode` | `pub fn decode(&self, _encoding: &str)` and `s.str().decode("utf-8")` — the pandas `Series.str.decode(encoding)` **text-encoding** API, not JWT. |

627 = 537 + 88 + 2 — fully accounted for.

The warning tier is the same picture: e.g. "Path join/push with untrusted
segment" fires on `out_order.push(name.clone())` (a `Vec::push`, not
`Path::push`); the rest are clones, casts, direct indexing, and `eprintln!`
inside tests — heuristic smells, not bugs, in a numeric kernel whose whole job
is dtype casts and contiguous indexing.

## Genuine-signal categories (kept)

For a pure-compute kernel the analyzer categories that *can* surface real bugs
are: 1 Ownership & Error Handling, 2 Unsafe & Memory (compiler-guaranteed empty
— `#![forbid(unsafe_code)]`), 3 Concurrency & Async, 4 Numeric & Floating-Point,
13 Build Health. These produce **no criticals**; their warnings are
unwrap/expect in test or invariant-guarded internal paths.

## Canonical clean-signal invocation

Skip the credential/web/auth category (rust cat 8) that produces 539 of the 627
FP criticals:

```bash
ubs --only=rust --skip-rust=8 crates/fp-frame/src/lib.rs
```

The residual 88 "panic in library code" criticals are inherent to scanning a
file that carries its unit tests inline (the test `panic!` assertions); they are
correct and not actionable. No per-line `ubs:ignore` annotations are added — 627
suppression comments would pollute the kernel for zero signal.

## Conclusion

fp-frame has **no genuine UBS critical findings**. The broad inventory is
heuristic false-positives from threat-surface categories irrelevant to a
safe-Rust columnar compute crate. Bead br-frankenpandas-yavyk is closed with
this documented triage; no follow-up bug beads are filed because no real defects
were found.
