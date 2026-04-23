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

Additional agents (`cmi`, review agents, etc.) join temporarily via
the NTM swarm orchestrator; their commits carry `Co-Authored-By:`
footers attributing the specific model variant.

### Signing key fingerprints

*(Populated once br-frankenpandas-3d5q policy decision is made and
the maintainer + each agent publishes an SSH signing key. Until then,
commit-signature verification is absent — this is tracked under
3d5q itself.)*

| Identity | SSH key fingerprint | Notes |
|----------|--------------------|-------|
| Jeffrey Emanuel | *(pending)* | Primary human maintainer; required for release tags. |
| cc-pandas | *(pending)* | Per-session key; rotated on account changes. |
| cod-pandas | *(pending)* | Codex-identity key; separate per-session. |

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
