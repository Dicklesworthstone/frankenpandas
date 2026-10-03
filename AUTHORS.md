# Authors

frankenpandas is developed by Jeffrey Emanuel with assistance from a
multi-agent AI coding swarm. This file records the agent identities
contributing commits, what each agent's role is, and (when applicable)
the SSH signing key fingerprints we expect to see on commits from
each identity.

## Human maintainer

- **Jeffrey Emanuel** — project founder / maintainer / release manager.
  Git identities: `Dicklesworthstone <jeff141421@gmail.com>`.

## Swarm agents

All swarm agents commit under a role-specific name (`cc-pandas`,
`cod-pandas`, `Clawdstein-libupdater-frankenpandas`, etc.) with the
maintainer's email address. The `.mailmap` normalizes these back to
`Dicklesworthstone` for `git shortlog` / GitHub "Contributors" views,
so the contributor list stays clean. When commit signing lands per
the policy half of br-frankenpandas-3d5q, each agent identity will
publish its ed25519 public key below.

### Active agent identities

| Agent | Role | Typical work |
|-------|------|--------------|
| `cc-pandas` | Claude Code (this agent) | Review-mode audits, implementation of HIGH/MEDIUM beads, session handoffs. |
| `cod-pandas` | Codex (OpenAI o-series) | Large refactors (lxhr monolith split, SQL backend epic slices), conformance gate work. |
| `Clawdstein-libupdater-frankenpandas` | Claude Code specializing in dependency sweeps | Library-updater runs, asupersync bumps. |
| `cod-pandas-release-W4` | Codex release worker | Release qualification, dependency gates, and signed release tags. |

Additional agents (`cmi`, review agents, etc.) join temporarily via
the NTM swarm orchestrator; their commits carry `Co-Authored-By:`
footers attributing the specific model variant.

### Signing key fingerprints

The release worker's key is recorded below for explicit local verification.
Other identity keys and GitHub registration remain pending under
br-frankenpandas-3d5q; do not infer verification for those identities.

| Identity | SSH key fingerprint | Notes |
|----------|--------------------|-------|
| Jeffrey Emanuel | *(pending)* | Primary human maintainer's signing key. |
| cc-pandas | *(pending)* | Per-session key; rotated on account changes. |
| cod-pandas | *(pending)* | Codex-identity key; separate per-session. |
| cod-pandas-release-W4 | `SHA256:WjIbFn7vI825B3mtgATqM+mNQptwLWsr6PTLRE494+k` | Release signing key created 2026-10-03. GitHub signing-key registration is pending; verify local signatures against this fingerprint. |

## How to contribute

External human contributors (not swarm agents) are welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) — *(pending, tracked under
br-frankenpandas-6d5s)*.

## Attribution

Third-party code included under compatible licenses is tracked in
the [LICENSE](LICENSE) file. Python-side dependencies for the
conformance oracle are pinned in
[crates/fp-conformance/oracle/requirements.txt](crates/fp-conformance/oracle/requirements.txt).

## Contact

- Security issues: see [SECURITY.md](SECURITY.md) for private
  disclosure channel.
- General: open a GitHub issue using the templates in
  [.github/ISSUE_TEMPLATE/](.github/ISSUE_TEMPLATE/).
- Agent coordination: swarm agents file bead reports via the
  `br` CLI (see [AGENTS.md](AGENTS.md)), not GitHub issues.
