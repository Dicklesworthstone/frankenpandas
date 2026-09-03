# FrankenPandas Release Runbook (signed tags, 0.3.0+)

> Operational runbook for cutting a release. Written under `br-frankenpandas-rc-signed-release-030-kf1lc` (reality-check 2026-09-03). Everything here is agent-executable EXCEPT the steps marked **[MAINTAINER]** — those need the release manager's signing key / crates.io token.

## 0. Release gating (what must be green before tagging)

| Gate | Why | Evidence |
|---|---|---|
| CI green batch | README's publish story is "cut the next release from a green CI batch"; CI was producing zero successes (br-frankenpandas-ey5sl) — do not tag until a full `ci.yml` run is green | GH Actions run URL |
| Live-oracle report exists and is honest | `artifacts/ci/live_oracle_report.json` (br-frankenpandas-rc-live-oracle-local-run-8oey9) — the crate page repeats the parity number; it must be reproducible | report file + HEAD sha inside |
| Packaging landmine closed | `br-frankenpandas-rc-sse41-downstream-lock-hrnom`: `fp-columnar` fails optimized builds without a `+sse4.1` stanza. Consumers MUST have a documented path (README "Building release binaries that depend on frankenpandas") BEFORE the next crates.io publish, or every downstream release build breaks | consumer probe in the bead |
| Packet corpus green | `python3 scripts/gen_feature_parity_table.py --check` (0 failing; pending entries must be explained orphans) | docs/planning/FEATURE_PARITY.md |
| `cargo package --list` clean for all 15 crates | no stray artifacts in the shipped tars | `cargo package --list -p <crate>` per crate |

## 1. Version bump (workspace single-source)

1. `Cargo.toml [workspace] package.version` is the single source (all crates inherit `version.workspace = true`, per br-h8a8). Bump it once (e.g. `0.2.0 → 0.3.0`).
2. Update the `CHANGELOG.md` header line "Workspace version is **0.2.0**" and add/refresh the version-timeline row.
3. Commit: `chore(release): bump workspace version to 0.3.0 [br-frankenpandas-rc-signed-release-030-kf1lc]`.

## 2. Tag — **[MAINTAINER]** (one-time key setup, then mechanical)

One-time (per AGENTS.md "Commit provenance"):

```bash
ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519_release
git config --local user.signingkey ~/.ssh/id_ed25519_release.pub
git config --local gpg.format ssh
git config --local tag.gpgsign true     # tags only; commit signing policy is 3d5q's
```

Register the public key with GitHub as a **Signing Key**, then publish its fingerprint into `AUTHORS.md` (the table currently carries no keys — that is the pending half of 3d5q).

Per release:

```bash
git tag -s frankenpandas-v0.3.0 -m "frankenpandas 0.3.0"
git tag -s v0.3.0 -m "workspace 0.3.0"                      # historic dual-tag convention, see CHANGELOG timeline
git push origin main && git push origin main:master          # master sync is mandatory per AGENTS.md
git push origin frankenpandas-v0.3.0 v0.3.0
```

Verify the chain (AGENTS.md "Verifying a commit locally"):

```bash
git tag -v frankenpandas-v0.3.0          # or: git log --show-signature -1 frankenpandas-v0.3.0
git log --format='%G?' -1 frankenpandas-v0.3.0   # expect G (good) or U (good, unknown key)
```

## 3. Publish

Primary path is automated: `.github/workflows/release-plz.yml` (config `release-plz.toml`) opens a release PR from conventional commits; merging it triggers the release job, which tags and — once the maintainer flips `publish = true` (currently `false` by design, br-4clx) — publishes crates in **topological dependency order**.

Manual fallback (only if release-plz is broken), in dependency order:

```
fp-types → fp-columnar → fp-dot-kernel → fp-index → fp-runtime → fp-expr
→ fp-frame → fp-groupby → fp-join → fp-io → fp-conformance → fp-bench
→ fp-frankentui → fp-python → frankenpandas
```

```bash
cargo publish -p fp-types            # repeat per crate, order above; use --dry-run first
cargo publish --dry-run -p <crate>   # for every member before the real pass
```

`cargo publish` is **[MAINTAINER]** (crates.io token).

## 4. Post-publish consumer verification (the probe that must pass)

From OUTSIDE the workspace, a stock consumer must build in release via the documented path:

```toml
# consumer Cargo.toml
cargo-features = ["profile-rustflags"]        # nightly cargo; first line
[dependencies]
frankenpandas = "=0.3.0"
[profile.release.package.fp-columnar]
rustflags = ["-Ctarget-feature=+sse4.1"]
```

`cargo build --release` must succeed; REMOVING the stanza must fail with the `E0080` message that points at README "Building release binaries that depend on frankenpandas". Both directions are the `hrnom` probe.

## 5. Immediately after

- README Roadmap "Release to crates.io" row: new version, date, signed-tag note.
- CHANGELOG timeline row with the tag link.
- Confirm `master` == `main` on the remote (`git rev-parse origin/main origin/master`).
