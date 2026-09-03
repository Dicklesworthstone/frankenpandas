use std::{
    fs,
    path::{Path, PathBuf},
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fuzz_target_names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root.join("fuzz/fuzz_targets"))
        .expect("read fuzz targets")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    names
}

#[test]
fn ci_workflow_runs_supply_chain_security_scans() {
    let root = repo_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci");
    assert!(ci.contains("security:"), "expected security job in ci.yml");
    assert!(ci.contains("licenses:"), "expected licenses job in ci.yml");
    assert!(
        ci.contains("cargo install cargo-audit --locked"),
        "expected cargo-audit install step in ci.yml"
    );
    assert!(
        ci.contains("cargo audit --deny warnings"),
        "expected cargo-audit execution in ci.yml"
    );
    assert!(
        ci.contains("cargo install cargo-deny --locked"),
        "expected cargo-deny install step in ci.yml"
    );
    assert!(
        ci.contains("cargo deny check advisories bans licenses sources"),
        "expected cargo-deny policy execution in ci.yml"
    );
}

#[test]
fn dependabot_tracks_cargo_and_actions_weekly() {
    let root = repo_root();
    let dependabot =
        fs::read_to_string(root.join(".github/dependabot.yml")).expect("read dependabot");
    assert!(
        dependabot.contains("package-ecosystem: \"cargo\""),
        "expected cargo ecosystem updates in dependabot config"
    );
    assert!(
        dependabot.contains("package-ecosystem: \"github-actions\""),
        "expected github-actions updates in dependabot config"
    );
    assert!(
        dependabot.contains("interval: \"weekly\""),
        "expected weekly cadence in dependabot config"
    );
    assert!(
        dependabot.contains("rust-deps:"),
        "expected grouped cargo updates in dependabot config"
    );
    assert!(
        dependabot.contains("github-actions:"),
        "expected grouped GitHub Actions updates in dependabot config"
    );
}

#[test]
fn deny_toml_locks_license_and_source_policy() {
    let root = repo_root();
    let deny = fs::read_to_string(root.join("deny.toml")).expect("read deny.toml");
    assert!(
        deny.contains("allow-registry = [\"https://github.com/rust-lang/crates.io-index\"]"),
        "expected crates.io-only source policy in deny.toml"
    );
    assert!(
        deny.contains("\"MIT\"") && deny.contains("\"Apache-2.0\""),
        "expected baseline permissive license allowlist in deny.toml"
    );
    assert!(
        deny.contains("unknown-git = \"deny\""),
        "expected unknown git sources to be denied in deny.toml"
    );
}

#[test]
fn cargo_lock_excludes_tokio_runtime_family() {
    let root = repo_root();
    let lock = fs::read_to_string(root.join("Cargo.lock")).expect("read Cargo.lock");
    for forbidden in [
        "name = \"tokio\"",
        "name = \"tokio-macros\"",
        "name = \"tokio-postgres\"",
        "name = \"tokio-util\"",
    ] {
        assert!(
            !lock.contains(forbidden),
            "workspace no-Tokio policy violation: Cargo.lock contains {forbidden}"
        );
    }
}

#[test]
fn fuzz_targets_have_committed_regression_corpus_and_artifact_dirs() {
    let root = repo_root();
    let targets = fuzz_target_names(&root);
    assert!(!targets.is_empty(), "expected at least one fuzz target");

    for target in targets {
        let corpus_dir = root.join("fuzz/corpus").join(&target);
        assert!(corpus_dir.is_dir(), "missing corpus dir for {target}");

        let seed_count = fs::read_dir(&corpus_dir)
            .expect("read corpus dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .count();
        assert!(
            seed_count >= 4,
            "expected at least four committed seeds for {target}, found {seed_count}"
        );

        let artifact_readme = root.join("fuzz/artifacts").join(&target).join("README.md");
        assert!(
            artifact_readme.is_file(),
            "missing artifact README for {target}: {}",
            artifact_readme.display()
        );
    }
}

#[test]
fn ci_workflows_lock_in_fuzz_regressions() {
    let root = repo_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci");
    assert!(
        ci.contains("fuzz-regression"),
        "expected fuzz-regression job in ci.yml"
    );
    assert!(
        ci.contains("cargo fuzz run"),
        "expected cargo fuzz replay command in ci.yml"
    );
    assert!(
        ci.contains("corpus/$target"),
        "expected committed fuzz corpus replay in ci.yml"
    );

    let nightly = fs::read_to_string(root.join(".github/workflows/fuzz-nightly.yml"))
        .expect("read nightly fuzz workflow");
    assert!(
        nightly.contains("schedule:"),
        "expected nightly fuzz workflow schedule"
    );
    assert!(
        nightly.contains("-max_total_time=60"),
        "expected nightly fuzz workflow to spend real time mutating"
    );
}

#[test]
fn ci_workflow_runs_perf_regression_gate_instead_of_noop_bench() {
    let root = repo_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci");
    assert!(
        ci.contains("Performance regression gate"),
        "expected named performance regression gate in ci.yml"
    );
    assert!(
        ci.contains("cargo test -p fp-conformance --test perf_baselines -- --nocapture --ignored --skip perf_run_all_baselines"),
        "expected CI to run ignored perf_baselines with the summary case skipped"
    );
    assert!(
        !ci.contains("run: cargo bench"),
        "expected CI to stop using the no-op cargo bench smoke step"
    );
}

#[test]
fn rust_toolchain_is_date_pinned_with_required_components() {
    let root = repo_root();
    let toolchain =
        fs::read_to_string(root.join("rust-toolchain.toml")).expect("read rust-toolchain");
    // Deliberately asserts the exact pinned date: bumping the toolchain must be
    // a conscious edit here too, so the pin cannot drift silently.
    assert!(
        toolchain.contains("channel = \"nightly-2026-08-25\""),
        "expected rust-toolchain.toml to pin an exact nightly date"
    );
    assert!(
        toolchain.contains("components = [\"rustfmt\", \"clippy\", \"rust-src\"]"),
        "expected rust-toolchain.toml to pin rustfmt, clippy, and rust-src"
    );
}

#[test]
fn ci_workflow_uses_pinned_rust_toolchain_from_file() {
    let root = repo_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci");
    assert!(
        ci.contains("Resolve pinned Rust toolchain"),
        "expected ci.yml to resolve the pinned toolchain from rust-toolchain.toml"
    );
    assert!(
        ci.contains("print(f\"channel={tomllib.load(fh)['toolchain']['channel']}\")"),
        "expected ci.yml to read the toolchain channel from rust-toolchain.toml"
    );
    assert!(
        ci.contains("uses: dtolnay/rust-toolchain@master"),
        "expected ci.yml to use dtolnay/rust-toolchain@master for explicit toolchain inputs"
    );
    assert!(
        ci.contains("toolchain: ${{ steps.rust_toolchain.outputs.channel }}"),
        "expected ci.yml to pass the resolved pinned toolchain into setup steps"
    );
    assert!(
        !ci.contains("uses: dtolnay/rust-toolchain@nightly"),
        "expected ci.yml to stop floating the GitHub Action nightly ref"
    );
}

#[test]
fn ci_workflow_runs_workspace_rustdoc_gate() {
    let root = repo_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci");
    assert!(
        ci.contains("Rustdoc"),
        "expected named Rustdoc step in ci.yml"
    );
    assert!(
        ci.contains("RUSTDOCFLAGS: -D warnings"),
        "expected rustdoc warnings to be denied in ci.yml"
    );
    assert!(
        ci.contains("cargo doc --workspace --no-deps --all-features"),
        "expected CI to build workspace docs with all features"
    );
}

#[test]
fn ci_workflow_has_a_non_advisory_core_test_gate_and_required_oracle() {
    let root = repo_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci");
    let core_gate = ci
        .split("  core-test:\n")
        .nth(1)
        .and_then(|section| section.split("\n  eager-frame:").next())
        .expect("expected standalone core-test job before eager-frame");

    assert!(
        core_gate.contains("cargo test -p fp-frame -p fp-columnar --no-fail-fast"),
        "expected core-test to execute both fp-frame and fp-columnar suites"
    );
    assert!(
        !core_gate
            .lines()
            .any(|line| line.trim_start().starts_with("continue-on-error:")),
        "core-test must fail the workflow instead of reporting an advisory result"
    );
    assert!(
        ci.contains("FP_PYTHON_BIN: .venv-oracle/bin/python"),
        "expected CI to use its isolated, pinned live-oracle interpreter"
    );
    assert!(
        ci.contains("required live pandas oracle interpreter is missing"),
        "expected a clear hard failure when the required live oracle is absent"
    );
}

/// README numbers must match the tree they describe (br-frankenpandas-rc-readme-number-truth-mzox7).
///
/// Recomputes the measured quantities and asserts the README documents exactly
/// them. Negative case a naive doc-lint would miss: each check fails when the
/// TREE changes without a README update (the drift direction this gate exists
/// for) and equally when the README inflates without a tree change.
#[test]
fn readme_documented_numbers_match_the_tree() {
    fn walk(dir: &Path, pred: &mut impl FnMut(&Path) -> bool) -> usize {
        let mut n = 0;
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                n += walk(&path, pred);
            } else if pred(&path) {
                n += 1;
            }
        }
        n
    }
    fn count_files(dir: &Path, suffix: &str) -> usize {
        walk(dir, &mut |p| p.extension().and_then(|e| e.to_str()) == Some(suffix))
    }
    fn thousands(n: usize) -> String {
        let s = n.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        out
    }
    fn grep_count(root: &Path, rel: &str, needle: &str) -> usize {
        walk(&root.join(rel), &mut |p| {
            p.extension().and_then(|e| e.to_str()) == Some("rs")
                && fs::read_to_string(p).map(|s| s.contains(needle)).unwrap_or(false)
        })
    }

    let root = repo_root();
    let readme = fs::read_to_string(root.join("README.md")).expect("read README");

    // (1) Packet corpus size.
    let packets = count_files(&root.join("crates/fp-conformance/fixtures/packets"), "json");
    assert!(
        readme.contains(&format!("{} packet JSON files", thousands(packets))),
        "README packet count does not match tree ({packets}); update README or the corpus"
    );

    // (2) Fixtures-tree JSON total (packets + adversarial + side-set).
    let fixture_json = count_files(&root.join("crates/fp-conformance/fixtures"), "json");
    assert!(
        readme.contains(&format!("{} fixture files", thousands(fixture_json))),
        "README fixture-file count does not match tree ({fixture_json})"
    );
    // (3) thread::scope fan-out occurrences under crates/*/src (the metric the
    // README states; files-with-hits and spawn-call counts are different
    // denominators and drift differently).
    fn count_matches(dir: &Path, needle: &str) -> usize {
        let mut n = 0;
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                n += count_matches(&path, needle);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
                && path.to_string_lossy().contains("/src/")
            {
                n += fs::read_to_string(&path)
                    .map(|s| s.matches(needle).count())
                    .unwrap_or(0);
            }
        }
        n
    }
    let scope_sites = count_matches(&root.join("crates"), "thread::scope");
    assert!(
        readme.contains(&format!("{} occurrences across", scope_sites)),
        "README thread::scope occurrence count does not match tree ({scope_sites})"
    );

    // (4) DISCREPANCIES section counts (README's own claim format).
    let disc = fs::read_to_string(root.join("crates/fp-conformance/DISCREPANCIES.md"))
        .expect("read DISCREPANCIES");
    let total = disc.matches("\n### DISC-").count();
    let active = disc
        .split("## Resolved Divergences")
        .next()
        .map(|head| head.matches("\n### DISC-").count())
        .unwrap_or(0);
    assert_eq!(total, 26, "DISCREPANCIES entry count changed; update README + this gate");
    assert_eq!(active, 15, "DISCREPANCIES active-section count changed; update README + this gate");
    assert!(
        readme.contains(&format!("{} numbered divergence entries ({} active", total, active)),
        "README DISC counts do not match DISCREPANCIES.md ({total} total / {active} active)"
    );
}
