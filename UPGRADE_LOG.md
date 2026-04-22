# Dependency Upgrade Log

Date of first entry: 2026-04-21
Most recent sweep: 2026-04-22 (Clawdstein-libupdater-frankenpandas)
Most recent verification: 2026-04-22 10:27Z (cc-pandas)

## Verification (2026-04-22 10:27Z sweep, cc-pandas)

Exhaustive recheck across all 22 tracked dependencies in
`Cargo.toml` workspace.dependencies and every per-crate `fp-*/Cargo.toml`
pin — queried `https://crates.io/api/v1/crates/<name>` for each and
compared `max_stable_version` against the current pin. Result: **all
22 match crates.io max stable** (asupersync 0.3.1 confirmed in
`crates/fp-runtime/Cargo.toml`; Cargo.lock carries the matching
franken-kernel / franken-evidence / franken-decision 0.3.1 triple).
No further bumps available without hitting pre-release tracks.

Baseline build: `cargo check --workspace --all-targets` under rch →
exit 0. `cargo test -p fp-conformance --lib` → 373 passed / 0 failed
(improvement from the 334/18 recorded in the previous sweep — the
18 env-dependent sidecar failures resolved in the intervening fixture
work).

`serde_yaml` remains flagged as needs-attention: upstream archived,
no newer stable under that name. Migration candidates (`serde_yml`,
`serde_yaml_ng`) deferred.



## Summary (2026-04-22 sweep)

- **Updated:** 15 — asupersync (0.3.0 → 0.3.1, pre-workflow focused commit),
  libfuzzer-sys, bytes, bumpalo, regex, serde, serde_json, thiserror, raptorq,
  proptest, tempfile, sha2 (SemVer-major with call-site fix), calamine (8-minor
  jump), arrow + parquet (4 major versions, lockstep).
- **Skipped (already at latest):** chrono 0.4.44, chrono-tz 0.10.4, csv 1.4.0,
  rust_xlsxwriter 0.94.0, rusqlite 0.39.0, unicode-casefold 0.2.0.
- **Needs attention:** `serde_yaml` is published as
  `0.9.34+deprecated`; upstream archived. No newer stable under that name.
  Flagged for a dedicated future migration to `serde_yml` or `serde_yaml_ng`.
- **Failed:** 0.
- **Pre-existing baseline issue (NOT caused by this sweep):**
  `cargo test -p fp-conformance --lib` reports 18 FAILED / 334 passed on
  `main` prior to and after every bump. The 18 failures are env-dependent
  conformance-gate / packet-filter / CI-pipeline tests that rely on sidecar
  artifacts. Verified baseline via stash/unstash diff with every risky bump
  (sha2, arrow, parquet) — the 334/18 split is stable regardless of
  dependency version.

## Updates

### asupersync: 0.2.0 → 0.3.0 — 2026-04-21
- **Scope:** `crates/fp-runtime/Cargo.toml` (optional dep, `default-features = false`, gated by the `fp-runtime/asupersync` feature). `Cargo.lock` refreshed.
- **Landed in:** upstream commit `9e8b574`, which beat cc's parallel bump to origin; cc's identical 0.2 → 0.3 edit was dropped during rebase.
- **Breaking review:** The only source-breaking change on the 0.2.x → 0.3.0 path was v0.2.9 widening `ObjectParams.source_blocks` from `u8` to `u16` — not touched by fp-runtime. The v0.3.0 delta is overwhelmingly a coordinated dependency refresh (digest-0.11 wave, hashbrown 0.17, rusqlite 0.39, lz4_flex 0.13, signal-hook 0.4, rayon 1.12) plus three concurrency bug fixes (parking_lot self-deadlock in observability, DNS-coalesce waiter count, TLS close_notify fail-closed). Our only public-API touchpoint is `asupersync::Outcome<T, E>` (variants `Ok`/`Err`/`Cancelled`/`Panicked`), still exported from the crate root in v0.3.0 with the same shape.
- **Migration:** none. fp-runtime call sites in `src/lib.rs` (`outcome_to_action`) compile unchanged.
- **Tests:** `rch exec -- cargo test -p fp-runtime --features asupersync` → 30 passed, 0 failed. `rch exec -- cargo clippy -p fp-runtime --features asupersync --all-targets -- -D warnings` → clean. `rch exec -- cargo check -p fp-runtime --features asupersync` → OK.

### asupersync: 0.3.0 → 0.3.1 — 2026-04-22 (commit `54f9526`)
- **Scope:** `crates/fp-runtime/Cargo.toml` (flags preserved). Cargo.lock also picked up franken-kernel / franken-evidence / franken-decision 0.3.0 → 0.3.1 (transitive, version-locked with asupersync).
- **Breaking:** none (patch release, pulled within ~3h of crates.io publish).
- **Tests:** `cargo check -p fp-runtime --features asupersync --all-targets` → OK; `cargo test -p fp-runtime --features asupersync` → 30 passed, 0 failed.
- **Note:** A parallel agent landed the same logical bump as commit `a0b05d2` immediately after; resolved via rebase, no conflicts.

### libfuzzer-sys: 0.4.10 → 0.4.12 — 2026-04-22 (commit `b3f3322`)
- **Scope:** `fuzz/Cargo.toml` (separate inner workspace).
- **Breaking:** none.
- **Tests:** not built (fuzz requires nightly + `cargo-fuzz`); patch-level change, negligible risk.

### bytes: 1.10.1 → 1.11.1 — 2026-04-22 (commit `410b673`)
- **Scope:** `[workspace.dependencies]` floor bump. Cargo.lock already at 1.11.1 transitively.
- **Tests:** `cargo test -p fp-io --lib` → 192 passed, 0 failed.

### bumpalo: 3.16.0 → 3.20.2 — 2026-04-22 (commit `52241ca`)
- **Scope:** `[workspace.dependencies]` floor bump. Used by fp-groupby / fp-join arena allocators.
- **Tests:** `cargo test -p fp-groupby --lib` → 65 passed, 0 failed.

### regex: 1.11.1 → 1.12.3 — 2026-04-22 (commit `9a3b962`)
- **Scope:** `[workspace.dependencies]` floor bump. regex 1.x guarantees non-breaking; breaking changes reserved for 2.0.
- **Tests:** `cargo test -p fp-frame --lib` → 1382 passed, 0 failed.

### serde: 1.0.219 → 1.0.228 — 2026-04-22 (commit `063f238`)
- **Scope:** `[workspace.dependencies]` floor bump. Patch-level.
- **Tests:** `cargo test -p fp-types --lib` → 62 passed, 0 failed.

### serde_json: 1.0.140 → 1.0.149 — 2026-04-22 (commit `06fc0ec`)
- **Scope:** `[workspace.dependencies]` floor bump (`preserve_order` feature retained). Patch-level.
- **Tests:** `cargo test -p fp-io --lib` → 192 passed, 0 failed.

### thiserror: 2.0.12 → 2.0.18 — 2026-04-22 (commit `62c258b`)
- **Scope:** `[workspace.dependencies]` floor bump. Patch-level.
- **Tests:** `cargo test -p fp-io --lib` → 192 passed, 0 failed.

### raptorq: 2.0.0 → 2.0.1 — 2026-04-22 (commit `f58d9b6`)
- **Scope:** `crates/fp-conformance/Cargo.toml` (erasure-coded packet envelopes).
- **Tests:** `cargo test -p fp-conformance --lib` → 334 passed, 18 failed (baseline unchanged; verified pre-existing via stash/unstash).

### proptest: 1.10.0 → 1.11.0 — 2026-04-22 (commit `bbccfda`)
- **Scope:** `crates/fp-conformance/Cargo.toml` + `crates/fp-frankentui/Cargo.toml` (both dev-deps).
- **Tests:** baseline 334/18 unchanged.

### tempfile: 3.14.0 → 3.27.0 — 2026-04-22 (commit `c35d33f`)
- **Scope:** `crates/fp-conformance/Cargo.toml` + `crates/fp-frankentui/Cargo.toml` (both dev-deps). 3.14 → 3.27 is additive (new `env::override_temp`, `Builder::disable_cleanup`); call sites using `TempDir::new` / `Builder::prefix().tempdir()` / `NamedTempFile::new` are compatible.
- **Tests:** baseline 334/18 unchanged.

### sha2: 0.10.9 → 0.11.0 (SemVer-major for 0.x) — 2026-04-22 (commit `0f254f4`)
- **Scope:** `crates/fp-conformance/Cargo.toml` + two call-site edits in `crates/fp-conformance/src/lib.rs`.
- **Breaking:** `Sha256::digest` return type renamed `GenericArray<u8, U32>` → `Array<u8, U32>` (hybrid-array crate). The new `Array` type deliberately removed the `std::fmt::LowerHex` impl, so `format!("{:x}", digest)` no longer compiles.
- **Migration:** rewrote the two call sites:
  - `hash_bytes`: `format!("{:x}", Sha256::digest(bytes))` → `hex_encode(Sha256::digest(bytes).as_slice())`
  - `ArtifactId::short_hash`: `format!("{:x}", hash)[..8].to_owned()` → `hex_encode(hash.as_slice())[..8].to_owned()`
  - Reused the existing `hex_encode` helper (lowercase hex per byte) so textual output is bit-identical to `{:x}`, preserving sidecar / fixture / packet-id compatibility.
- **Tests:** baseline 334/18 unchanged (hash format parity confirmed by test count preservation).
- **Note:** Cargo.lock hosts sha2 0.10.9 AND 0.11.0 simultaneously; 0.10 still pulled by asupersync, 0.11 used by fp-conformance.

### calamine: 0.26.1 → 0.34.0 (8-minor jump) — 2026-04-22 (commit `2f9d325`)
- **Scope:** `[workspace.dependencies]` floor bump; fp-io is the only consumer.
- **Transitive refresh:** quick-xml 0.31 → 0.39.2, + atoi_simd / debug_unsafe / fast-float2; removed zip 2.4 (backend switch), arbitrary / derive_arbitrary 1.4, displaydoc 0.2.
- **Public API used by fp-io:** `calamine::{Data, Reader, open_workbook_auto, open_workbook_auto_from_rs}`; `Data` enum variants (Int, Float, String, Bool, Empty, DateTime, DateTimeIso, DurationIso, Error) — all stable across the jump.
- **Tests:** `cargo test -p fp-io --lib` → 192 passed, 0 failed.

### arrow + parquet: 54.3.0 → 58.1.0 (4 major versions) — 2026-04-22 (commit `795cdd0`)
- **Scope:** `[workspace.dependencies]` floor bumps for both. Pulled in lockstep.
- **Transitive refresh:** 12 arrow sub-crates in lockstep; parquet 58.1; flatbuffers 24 → 25; twox-hash 1.6 → 2.1; removed legacy num 0.4 family, bitflags 1.x, static_assertions.
- **Features preserved:** arrow `["prettyprint", "ipc"]` / parquet `["arrow", "snap"]`.
- **Public API used by fp-io — all stable across 54 → 58:**
  - `arrow::array::{Array, Int32Array, Float32Array, LargeStringArray, Int64Array}`
  - `arrow::datatypes::{DataType, Field, Schema, TimeUnit, Date32Type, Date64Type, TimestampSecondType/Milli/Micro/Nano}`
  - `arrow::temporal_conversions::{as_date, as_datetime}`
  - `arrow::ipc::{writer::FileWriter, writer::StreamWriter, reader::FileReader, reader::StreamReader}`
  - `arrow::error::ArrowError`
  - `parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder}`
- **Tests:** `cargo test -p fp-io --lib` → 192 passed, 0 failed (Parquet round-trip, Feather, IPC stream/file, Arrow dtype inference and datetime conversion all green). `cargo test -p fp-conformance --lib` → baseline 334/18 unchanged.

## Needs Attention (for human follow-up)

### serde_yaml: 0.9.34+deprecated (upstream archived)
- **Issue:** `serde_yaml` 0.9.34 is the final release; the crate is officially deprecated.
- **Recommended migration paths:**
  - `serde_yml` (community continuation, drop-in replacement for most cases)
  - `serde_yaml_ng` (another maintained fork)
- **Blast radius:** `crates/fp-conformance/Cargo.toml` line 25 only; used in fp-conformance's YAML sidecar parsing.
- **Skipped in this sweep:** no newer stable exists under the `serde_yaml` name, so nothing to bump. Flagged for a dedicated migration commit.
