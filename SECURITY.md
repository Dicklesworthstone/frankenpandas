# Security Policy

## Supported Versions

The latest published release of every crate in this workspace receives
security updates. Pre-release tags (`0.y.z` before 1.0.0 stabilization)
are patched on a best-effort basis for the immediately-previous minor;
older minors are not backported.

## Reporting a Vulnerability

**Do not file public issues for vulnerabilities.** Public disclosure
before a coordinated patch exposes every downstream user.

Instead, report privately via GitHub's Private Vulnerability Reporting:

- <https://github.com/Dicklesworthstone/frankenpandas/security/advisories/new>

(Enable Private Vulnerability Reporting in repo settings if you are
the maintainer; until that ticks on, reports to the maintainer's
email registered on crates.io are the fallback channel.)

Expected response timeline:
- **Acknowledgment within 48 hours**
- **Initial severity assessment within 7 days**
- **Coordinated disclosure within 90 days** (or earlier if upstream
  fixes land faster)

## Scope

### In scope
- Memory safety issues in frankenpandas-authored `unsafe` code
  (none exists today — every crate carries `#![forbid(unsafe_code)]`
  — but report anyway if you find our attribute removed).
- **Logic bugs in IO parsers** (fp-io: CSV, JSON, JSONL, Parquet,
  Excel, Feather, Arrow IPC, SQL) that allow crafted untrusted input
  to cause panic, OOM, unbounded memory growth, infinite loop,
  or integer overflow.
- **Logic bugs in expression evaluation** (fp-expr: `eval_str`,
  `query_str`, `parse_expr`) that allow crafted expression strings
  to crash or consume unbounded resources.
- **Logic bugs in SQL bindings** (fp-io SqlConnection trait) that
  allow crafted queries to panic the driver or bypass parameter
  binding.
- **Differential conformance divergences** that silently change
  numeric results vs pandas when the divergence affects scientific
  correctness (data corruption, not just dtype differences).

### Out of scope
- Behavior differences between frankenpandas and pandas that are
  documented in [DISCREPANCIES.md](crates/fp-conformance/DISCREPANCIES.md).
  These are intentional divergences.
- Issues in **transitive dependencies** (arrow, parquet, calamine,
  rusqlite, serde_yaml, etc.). Report upstream to the dep
  maintainer; we will accept a patch bump once upstream fixes and
  flag affected versions in our CHANGELOG.
- Denial-of-service via legitimate-but-large inputs (e.g. a 1 TB
  CSV file). Mitigate at the application layer by bounding input
  size before calling frankenpandas APIs.
- Issues requiring physical access or compromised credentials to
  the user's machine.

## Disclosure Philosophy

We practice **coordinated disclosure**:
1. Reporter files privately via GitHub Security Advisories.
2. We triage and draft a fix.
3. Reporter reviews the fix for correctness.
4. We publish a patched release with a RustSec Advisory ID.
5. Public disclosure happens simultaneously or after step 4.

## Acknowledgment

We credit reporters in the CHANGELOG under a `## Security` section
and in the published RustSec Advisory unless the reporter prefers
anonymity.

## Supply-Chain Posture

This workspace enforces (as of 2026-04 CI):
- `cargo audit --deny warnings` on every PR (br-frankenpandas-36qc).
- `cargo deny check advisories bans licenses sources` on every PR
  (br-frankenpandas-36qc).
- `#![forbid(unsafe_code)]` on every crate (verified).
- Fuzz regression corpus on every PR (br-frankenpandas-zjme).
- Differential conformance against live pandas on every PR
  (br-frankenpandas-d6xa).
- Dependabot weekly updates for Cargo + GitHub Actions.

These gates minimize silent introduction of known vulnerabilities.
They do NOT substitute for your report when you find something
novel — please report.

## Hall of Fame

*(Populated as valid reports arrive. See [CHANGELOG.md](CHANGELOG.md)
§ Security for per-release credits.)*
