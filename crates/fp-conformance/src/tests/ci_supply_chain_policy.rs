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
    assert!(
        toolchain.contains("channel = \"nightly-2026-04-22\""),
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
