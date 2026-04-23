use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
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
    let dependabot = fs::read_to_string(root.join(".github/dependabot.yml")).expect("read dependabot");
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
