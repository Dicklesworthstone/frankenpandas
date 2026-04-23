# Contributing to frankenpandas

Thanks for wanting to help. This project is a clean-room Rust
reimplementation of the pandas API, developed via a multi-agent AI
coding swarm with a human maintainer. Both human contributors and
swarm agents are welcome; a few conventions apply.

## Quick setup (fresh clone)

```bash
git clone https://github.com/Dicklesworthstone/frankenpandas.git
cd frankenpandas

# One-shot: point git at our committed hook directory.
./scripts/install-hooks.sh

# Build + test (uses the pinned nightly from rust-toolchain.toml).
cargo build --workspace
cargo test --workspace
```

## Required reading

Before submitting a PR, skim:

- [README.md](README.md) — API tour + positioning.
- [AGENTS.md](AGENTS.md) — swarm workflow for AI agents.
- [SECURITY.md](SECURITY.md) — vuln disclosure + in-scope surfaces.
- [AUTHORS.md](AUTHORS.md) — identity policy for swarm agents.
- [crates/fp-conformance/DISCREPANCIES.md](crates/fp-conformance/DISCREPANCIES.md) —
  documented intentional divergences from pandas.

## Filing an issue

**Do not open generic issues.** Use one of the four templates under
[.github/ISSUE_TEMPLATE/](.github/ISSUE_TEMPLATE/):

- Bug Report — defect in shipped frankenpandas code.
- Feature Request — request a new pandas API surface.
- Conformance Divergence — pandas-output-differs report (routes
  through DISCREPANCIES.md triage).
- Performance Regression — significant slowdown / memory regression.

For security vulnerabilities: use [GitHub Private Vulnerability Reporting](https://github.com/Dicklesworthstone/frankenpandas/security/advisories/new).
Never file public issues for vulnerabilities.

## Development workflow

### Human contributors

1. Fork the repo, create a feature branch.
2. Make changes; follow the style rules (see below).
3. Run `cargo test --workspace --all-targets` locally. Under
   [rch](https://crates.io/crates/rch) if you have it; otherwise
   plain cargo.
4. Run `cargo clippy --workspace --all-targets -- -D warnings` —
   zero warnings is enforced by CI.
5. Run `cargo fmt --check` — the repo's `rustfmt.toml` controls
   style.
6. Open a PR using the template at
   [.github/PULL_REQUEST_TEMPLATE.md](.github/PULL_REQUEST_TEMPLATE.md).
   All template fields are required:
   - Linked bead ID (`Closes: br-frankenpandas-XXXX`).
   - Pre-commit diff-stat (per br-iy8u doctrine).
   - Scope checklist (conformance / fuzz / perf / API-stability).
   - Testing checklist.

### Swarm agents

Set `AGENT_NAME=<agent-identity>` before commits (`cc-pandas`,
`cod-pandas`, etc. — see AUTHORS.md). File beads via `br create
--no-db --no-auto-flush --no-auto-import` — see AGENTS.md for the
full protocol.

## Style

### Rust source

- `rustfmt` with the settings in [`rustfmt.toml`](rustfmt.toml)
  (`edition = "2024"`, `max_width = 100`, imports_granularity Crate,
  group_imports StdExternalCrate).
- `cargo clippy --all-targets -- -D warnings` must pass.
- Every crate carries `#![forbid(unsafe_code)]`. Do not remove
  this attribute without a dedicated bead + explicit reviewer
  approval.
- Public error enums must carry `#[non_exhaustive]` (br-tne4).
  New variants ship as minor-version bumps, not major breaks.

### Non-Rust source

- YAML / Markdown / JSON / TOML: 2-space indent (per
  [`.editorconfig`](.editorconfig)).
- Python (oracle script / hooks): PEP 8, 4-space indent.
- Shell: `set -euo pipefail` at the top; LF line endings enforced
  by [`.gitattributes`](.gitattributes).

### Commit messages

Conventional-commit prefix + bead ID:
  `feat(fp-frame): <short summary> (br-frankenpandas-XXXX)`
  `fix(fp-io): <short summary> (br-frankenpandas-XXXX)`
  `test(fp-conformance): <short summary> (br-frankenpandas-XXXX)`
  `chore(beads): <short summary>`
  `docs: <short summary>`
  `ci: <short summary>`

Body paragraphs explain the "why" and list any cross-bead
interlocks. Footer for co-authored work: `Co-Authored-By: Name <email>`.

## Testing requirements per PR

| Change type | Must run locally | Must pass CI |
|---|---|---|
| fp-io parser / writer | `cargo test -p fp-io --lib` + `cargo test -p fp-conformance --lib` | `test` + `conformance` jobs |
| fp-frame core | `cargo test -p fp-frame --lib` + `cargo test -p fp-conformance --lib` | all CI jobs |
| fp-expr | `cargo test -p fp-expr --lib` | `test` + `conformance` |
| Fuzz harness | `cargo test -p fp-conformance --lib fuzz_` | `fuzz-regression` job |
| CI workflow changes | `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"` | all jobs |
| Docs only | N/A | N/A |

CI must be green before merge. Required jobs: `fmt`, `lint` (all
matrix cells), `test` (all matrix cells), `conformance`, `gates`,
`security`, `licenses`, `secret-scan`, `fuzz-regression`.

## Conformance against pandas

frankenpandas claims pandas-API parity verified by live differential
conformance. New public API surfaces that mirror pandas should ship
with:

1. Packet fixture(s) under `crates/fp-conformance/fixtures/packets/`
   capturing canonical input → pandas-oracle output pairs.
2. An inline live-oracle test (pattern: `live_oracle_<op>_matches_pandas`)
   OR a differential_* suite entry for bulk-fixture coverage.
3. DISCREPANCIES.md entry IF the divergence from pandas is intentional.

Divergences that slip through (user-reported via the Conformance
Divergence issue template) get triaged into DISCREPANCIES.md or
fixed.

## License + contribution terms

frankenpandas is licensed under MIT + OpenAI/Anthropic Rider (see
[LICENSE](LICENSE)). By submitting a PR, you agree your
contributions are licensed under the same terms. The dual
MIT / Apache-2.0 question is tracked under br-frankenpandas-dio8 and
will be revisited before 0.1.0 publishes to crates.io; until then,
contributions land under the current LICENSE.

## Code of Conduct

Be constructive and specific. Target behavior, not people. We're
building a library under an active AI-agent swarm — collaboration
expectations apply equally to human contributors and agent
identities.

If interactions go wrong, raise it in the SECURITY.md private
channel (interpreted broadly — same coordinated-disclosure flow
applies to interpersonal concerns as to technical vulns).

## Getting help

- Setup issues: open a GitHub issue using the Bug Report template.
- API questions: the rustdoc at docs.rs is the canonical reference
  (once 0.1.0 publishes).
- Swarm coordination: ping an active agent via `br show <bead-id>`
  — comment threads on beads are the project's coordination layer.
