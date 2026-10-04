# FrankenPandas Release Runbook (signed tags, 0.3.0+)

> Operational runbook for cutting a release. Written under `br-frankenpandas-rc-signed-release-030-kf1lc` (reality-check 2026-09-03). Everything here is agent-executable EXCEPT the steps marked **[MAINTAINER]** — those need the release manager's signing key / crates.io token.

## 0. Release gating (source before tagging, artifacts before upload)

| Gate | Why | Evidence |
|---|---|---|
| Pinned-toolchain gate | Run formatting, workspace/all-targets Clippy with warnings denied, and the full default workspace tests through RCH; release builds use DSR | Exact source revision and terminal RCH receipts |
| Live-oracle report exists and is honest | `artifacts/ci/live_oracle_report.json` (br-frankenpandas-rc-live-oracle-local-run-8oey9) — the crate page repeats the parity number; it must be reproducible | report file + HEAD sha inside |
| Portable release consumer | The 0.3.0 `fp-columnar` optimized-build lock was removed. An ordinary outside-workspace release consumer must build without a `+sse4.1` override; README documents optional x86_64 acceleration separately | Actual published-version consumer build and runtime receipt |
| Packet corpus green | `python3 scripts/gen_feature_parity_table.py --check` (0 failing; pending entries must be explained orphans) | docs/planning/FEATURE_PARITY.md |
| `cargo package --list` clean for all 15 crates | no stray artifacts in the shipped tars | `cargo package --list -p <crate>` per crate |
| Verified workspace packages, after tagging and before upload | The native DSR artifact producer runs `cargo package --workspace --locked -j1` in the exact tagged snapshot, compiling normalized archives against Cargo's temporary registry overlay | Terminal DSR receipt, unchanged post-build source audit and all 15 archive inventories |

The release-wave ruling permits disclosed pre-existing and non-default defects.
For 0.4.0 the stock G6 conformance step passed, while its stored-sidecar step
failed on missing reports also absent from the previous tag. The complete G6
pipeline remains nonzero; see [#41](https://github.com/Dicklesworthstone/frankenpandas/issues/41).
Do not infer positive live counts from an unavailable generated report.

## 1. Version bump (workspace single-source)

1. Bump `Cargo.toml [workspace.package] version` (all crates inherit `version.workspace = true`, per br-h8a8), all internal path-dependency version ranges, and the workspace lock entries together (e.g. `0.3.0 → 0.4.0`).
2. Update the `CHANGELOG.md` workspace version and add/refresh the version-timeline row.
3. Commit only the release's owned paths directly on `main` after qualification.

## 2. Tag — **[MAINTAINER]** (one-time key setup, then mechanical)

One-time (per AGENTS.md "Commit provenance"):

```bash
ssh-keygen -t ed25519 -f ~/.ssh/id_ed25519_release
git config --local user.signingkey ~/.ssh/id_ed25519_release.pub
git config --local gpg.format ssh
git config --local tag.gpgsign true     # tags only; commit signing policy is 3d5q's
```

Register the public key with GitHub as a **Signing Key** when authorized credentials permit, then publish its fingerprint and registration status in `AUTHORS.md`. Local signature verification must use an explicitly trusted public key even when GitHub registration is pending.

Per release:

```bash
git tag -s frankenpandas-v0.4.0 -m "frankenpandas 0.4.0"
git tag -s v0.4.0 -m "workspace 0.4.0"                      # historic dual-tag convention, see CHANGELOG timeline
git push --atomic origin main main:master frankenpandas-v0.4.0 v0.4.0
```

Verify the chain (AGENTS.md "Verifying a commit locally"):

```bash
git tag -v frankenpandas-v0.4.0          # with the trusted public key in allowed_signers
git log --format='%G?' -1 frankenpandas-v0.4.0   # this checks the commit signature separately
```

## 3. Publish

The release-wave path uses DSR for GitHub releases and direct crates.io uploads from the qualified source. Do not dispatch GitHub Actions for this path. Keep the existing `release-plz.toml` publication policy intact.

Publish the qualified source in dependency order, waiting for each new version to become available before its dependents:

```
fp-types → fp-dot-kernel → fp-columnar → fp-index → fp-runtime → fp-frame
→ fp-expr → fp-groupby → fp-join → fp-io → fp-conformance → fp-bench
→ fp-frankentui → fp-python → frankenpandas
```

```bash
cargo +nightly-2026-08-31 package --workspace --locked --allow-dirty -j1
cargo +nightly-2026-08-31 publish -p fp-types --locked --no-verify
```

Run verified packaging in the native DSR artifact producer's exact tagged
snapshot with a fresh target directory. Retain the terminal build receipt and
unchanged post-build source audit, and separately record custody of all 15
private crate archives. This is artifact-production proof, not an RCH gate.
Cargo repackages source during `publish`; before uploading, compare a
compiler-free `publish --dry-run --no-verify` archive with the qualified archive
from the identical non-Git source stage and Cargo version. Retain earlier target
directories. Repeat the upload command for each member in the order above.
`--no-verify` is permitted only after genuine archive verification; it must
never stand in for that gate. Keep the crates.io token in the publication
process environment and out of arguments, files and logs. `cargo publish` is
**[MAINTAINER]** (crates.io token).

## 4. Post-publish consumer verification (the probe that must pass)

From OUTSIDE the workspace, a stock consumer must build in release without
workspace-only profile overrides:

```toml
# consumer Cargo.toml
[dependencies]
frankenpandas = "=0.4.0"
```

`cargo build --release` must succeed. The previous release's `E0080` failure
without SSE4.1 is a regression control, not the desired behavior for 0.4.0.
The optional x86_64 `profile-rustflags` acceleration stanza is documented in
README "Building release binaries that depend on frankenpandas". Qualify the
published release's default build and real consumer operations separately
from the workspace's accelerated benchmark profile.

## 5. Immediately after

- README Roadmap "Release to crates.io" row: new version, date, signed-tag note.
- CHANGELOG timeline row with the tag link.
- Confirm `master` == `main` on the remote (`git rev-parse origin/main origin/master`).
