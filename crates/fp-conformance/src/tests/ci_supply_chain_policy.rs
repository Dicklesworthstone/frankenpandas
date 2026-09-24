use std::{
    collections::{BTreeMap, BTreeSet},
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
        toolchain.contains("channel = \"nightly-2026-08-31\""),
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

/// The quantities the README's headline numbers describe, measured from the tree.
#[derive(Debug, Clone)]
struct TreeCounts {
    packets: usize,
    fixture_json: usize,
    /// `thread::scope` occurrences under `crates/*/src` (the metric the README
    /// states; files-with-hits and spawn-call counts are different denominators
    /// and drift differently).
    scope_sites: usize,
    disc_total: usize,
    disc_active: usize,
    tests_src: usize,
    tests_all: usize,
    rust_lines_src: usize,
    rust_lines_all: usize,
    /// `crates/<name>/src` lines per crate directory name.
    crate_src_lines: BTreeMap<String, usize>,
}

impl TreeCounts {
    fn measure(root: &Path) -> Self {
        fn visit(dir: &Path, f: &mut impl FnMut(&Path)) {
            for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    visit(&path, f);
                } else {
                    f(&path);
                }
            }
        }
        fn count_json(dir: &Path) -> usize {
            let mut n = 0;
            visit(dir, &mut |p| {
                if p.extension().and_then(|e| e.to_str()) == Some("json") {
                    n += 1;
                }
            });
            n
        }
        let disc = fs::read_to_string(root.join("crates/fp-conformance/DISCREPANCIES.md"))
            .expect("read DISCREPANCIES");
        let mut counts = Self {
            packets: count_json(&root.join("crates/fp-conformance/fixtures/packets")),
            fixture_json: count_json(&root.join("crates/fp-conformance/fixtures")),
            scope_sites: 0,
            disc_total: disc.matches("\n### DISC-").count(),
            disc_active: disc
                .split("## Resolved Divergences")
                .next()
                .map_or(0, |head| head.matches("\n### DISC-").count()),
            tests_src: 0,
            tests_all: 0,
            rust_lines_src: 0,
            rust_lines_all: 0,
            crate_src_lines: BTreeMap::new(),
        };
        // Textual counts over every `.rs` file under crates/; lines are newline
        // counts, as `wc -l` reports them.
        let crates = root.join("crates");
        visit(&crates, &mut |p| {
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                return;
            }
            let bytes = fs::read(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
            let text = String::from_utf8_lossy(&bytes);
            let tests = text.matches("#[test]").count();
            let lines = text.matches('\n').count();
            counts.tests_all += tests;
            counts.rust_lines_all += lines;
            if p.to_string_lossy().contains("/src/") {
                counts.tests_src += tests;
                counts.rust_lines_src += lines;
                counts.scope_sites += text.matches("thread::scope").count();
            }
            let mut parts = p.strip_prefix(&crates).expect("under crates/").iter();
            if let (Some(name), Some(dir)) = (parts.next(), parts.next())
                && dir == "src"
            {
                *counts
                    .crate_src_lines
                    .entry(name.to_string_lossy().into_owned())
                    .or_default() += lines;
            }
        });
        counts
    }
}

/// A number written in `text` immediately before byte offset `end` ("1,387" in
/// "1,387 packets"): its value with thousands separators removed, and the byte
/// offset where it starts.
fn number_ending_at(text: &str, end: usize) -> Option<(usize, usize)> {
    let head = &text[..end];
    let start = head
        .char_indices()
        .rev()
        .find(|&(_, c)| !(c.is_ascii_digit() || c == ','))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let digits: String = head[start..].chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok().map(|n| (n, start))
}

/// One `<number><suffix>` claim in the README.
struct NumberClaim<'a> {
    line: usize,
    value: usize,
    /// Written as "more than <number>".
    floor: bool,
    /// The text after the suffix, which says what a floor counts.
    tail: &'a str,
}

fn number_claims<'a>(text: &'a str, suffix: &str) -> Vec<NumberClaim<'a>> {
    text.match_indices(suffix)
        .filter_map(|(at, _)| {
            let (value, start) = number_ending_at(text, at)?;
            Some(NumberClaim {
                line: text[..at].matches('\n').count() + 1,
                value,
                floor: text[..start].ends_with("more than "),
                tail: &text[at + suffix.len()..],
            })
        })
        .collect()
}

/// Every README number claim that disagrees with `tree`.
///
/// Exact counts must equal the tree in EVERY occurrence, not merely appear
/// once: the corpus reached 1,387 packets while four other copies still said
/// 1,341 and a presence check passed. Test and line counts are "more than N"
/// floors, which must not exceed the tree and must sit within 25% of it; an
/// understated floor ("5,000+ tests" when the tree had 8,700) misleads as much
/// as an inflated one. A claim family with no occurrence at all is reported,
/// so rewording the README cannot turn a check into a no-op.
fn readme_number_violations(readme: &str, tree: &TreeCounts) -> Vec<String> {
    /// A "more than `value`" floor is true of `have` and within 25% of it.
    const fn in_band(value: usize, have: usize) -> bool {
        value <= have && value * 4 >= have * 3
    }
    fn exact(
        out: &mut Vec<String>,
        readme: &str,
        label: &str,
        suffixes: &[&str],
        min: usize,
        want: usize,
    ) {
        let claims: Vec<NumberClaim<'_>> = suffixes
            .iter()
            .flat_map(|suffix| number_claims(readme, suffix))
            .filter(|claim| claim.value >= min)
            .collect();
        if claims.is_empty() {
            out.push(format!("no {label} claim found"));
        }
        for claim in claims {
            if claim.value != want {
                out.push(format!(
                    "line {}: {} {label}, the tree has {want}",
                    claim.line, claim.value
                ));
            }
        }
    }
    fn floor(out: &mut Vec<String>, readme: &str, suffix: &str, actual: impl Fn(&str) -> usize) {
        let claims = number_claims(readme, suffix);
        if claims.is_empty() {
            out.push(format!("no \"more than N{suffix}\" claim found"));
        }
        for claim in claims {
            let have = actual(claim.tail);
            if !claim.floor {
                out.push(format!(
                    "line {}: \"{}{suffix}\" should be a \"more than N\" floor (the tree has {have})",
                    claim.line, claim.value
                ));
            } else if !in_band(claim.value, have) {
                out.push(format!(
                    "line {}: \"more than {}{suffix}\" but the tree has {have}",
                    claim.line, claim.value
                ));
            }
        }
    }

    let mut out = Vec::new();
    exact(
        &mut out,
        readme,
        "packets",
        &[" packet JSON files", " packet files", " packets"],
        1_000,
        tree.packets,
    );
    exact(
        &mut out,
        readme,
        "fixture files",
        &[" fixture files"],
        0,
        tree.fixture_json,
    );
    exact(
        &mut out,
        readme,
        "thread::scope occurrences",
        &[" occurrences across"],
        0,
        tree.scope_sites,
    );
    exact(
        &mut out,
        readme,
        "DISCREPANCIES entries",
        &[
            " numbered divergence entries",
            " divergence entries",
            " numbered entries in `DISCREPANCIES.md`",
        ],
        0,
        tree.disc_total,
    );
    let active = format!(
        "{} numbered divergence entries ({} active",
        tree.disc_total, tree.disc_active
    );
    if !readme.contains(&active) {
        out.push(format!("no \"{active}\" claim found"));
    }
    floor(&mut out, readme, " `#[test]` markers", |tail| {
        if tail.starts_with(" under `crates/*/src`") {
            tree.tests_src
        } else {
            tree.tests_all
        }
    });
    floor(&mut out, readme, " lines of Rust under `src/`", |_| {
        tree.rust_lines_src
    });
    floor(
        &mut out,
        readme,
        " of those lines live under `src/`",
        |_| tree.rust_lines_src,
    );
    floor(&mut out, readme, " lines of Rust under `crates/`", |_| {
        tree.rust_lines_all
    });
    // Per-crate floors in the Workspace Structure tree
    // ("│   ├── fp-io/  # ... (more than 35,000 lines)"); every crate needs one.
    let mut listed = BTreeSet::new();
    for (i, text) in readme.lines().enumerate() {
        let Some((_, entry)) = text.split_once("── ") else {
            continue;
        };
        let Some((name, _)) = entry.split_once('/') else {
            continue;
        };
        let Some(&have) = tree.crate_src_lines.get(name) else {
            continue;
        };
        for claim in number_claims(text, " lines)") {
            listed.insert(name);
            if !claim.floor || !in_band(claim.value, have) {
                out.push(format!(
                    "line {}: {name} ({}{} lines) but crates/{name}/src has {have}",
                    i + 1,
                    if claim.floor { "more than " } else { "" },
                    claim.value
                ));
            }
        }
    }
    for name in tree.crate_src_lines.keys() {
        if !listed.contains(name.as_str()) {
            out.push(format!("no \"(more than N lines)\" floor for crate {name}"));
        }
    }
    out
}

/// README numbers must match the tree they describe
/// (br-frankenpandas-rc-readme-number-truth-mzox7, every-occurrence form
/// br-frankenpandas-rc0923-epic-truth-restoration-h5vo9.1). The check fails when
/// the TREE changes without a README update and equally when the README
/// inflates without a tree change.
#[test]
fn readme_documented_numbers_match_the_tree() {
    let root = repo_root();
    let readme = fs::read_to_string(root.join("README.md")).expect("read README");
    let tree = TreeCounts::measure(&root);
    assert_eq!(
        tree.disc_total, 28,
        "DISCREPANCIES entry count changed; update README + this gate"
    );
    assert_eq!(
        tree.disc_active, 16,
        "DISCREPANCIES active-section count changed; update README + this gate"
    );
    let violations = readme_number_violations(&readme, &tree);
    assert!(
        violations.is_empty(),
        "README numbers disagree with the tree {tree:?}:\n{}",
        violations.join("\n")
    );
}

/// The gate's negatives: a README right in one place and stale in another, an
/// understated floor, an inflated floor, a src-only floor judged against the
/// src count, a bare count where a floor belongs, a stale or missing per-crate
/// floor, and a README with the claim deleted are each reported; the
/// consistent README is not.
#[test]
fn readme_number_gate_reports_stale_copies_and_bad_floors() {
    let tree = TreeCounts {
        packets: 1_387,
        fixture_json: 1_400,
        scope_sites: 147,
        disc_total: 28,
        disc_active: 16,
        tests_src: 8_214,
        tests_all: 8_978,
        rust_lines_src: 570_359,
        rust_lines_all: 651_196,
        crate_src_lines: BTreeMap::from([
            ("fp-frame".to_owned(), 221_529),
            ("fp-io".to_owned(), 38_651),
        ]),
    };
    let good = "1,387 packet JSON files spanning 1,400 fixture files (147 occurrences across 8 files);\n\
        28 numbered divergence entries (16 active, the rest resolved);\n\
        more than 8,000 `#[test]` markers under `crates/*/src` and more than 8,500 `#[test]` markers across all of `crates/`;\n\
        more than 550,000 lines of Rust under `src/`; more than 600,000 lines of Rust under `crates/` (more than 550,000 of those lines live under `src/`).\n\
        │   ├── fp-frame/         # DataFrame, Series (more than 200,000 lines)\n\
        │   ├── fp-io/            # 14+ IO formats (more than 35,000 lines)\n\
        ├── artifacts/perf/       # Optimization round baselines (12 lines)\n";
    assert_eq!(readme_number_violations(good, &tree), Vec::<String>::new());

    let stale_copy = format!("{good}Adding a new packet (1,341 packet files and counting).\n");
    assert_eq!(
        readme_number_violations(&stale_copy, &tree),
        vec!["line 8: 1341 packets, the tree has 1387".to_owned()]
    );

    // The 2026-05 README called fp-frame "the 87,000-line crate".
    let stale_crate = good.replace("(more than 200,000 lines)", "(more than 87,000 lines)");
    assert_eq!(
        readme_number_violations(&stale_crate, &tree),
        vec![
            "line 5: fp-frame (more than 87000 lines) but crates/fp-frame/src has 221529"
                .to_owned()
        ]
    );
    let bare_crate = good.replace("(more than 35,000 lines)", "(38,651 lines)");
    assert_eq!(
        readme_number_violations(&bare_crate, &tree),
        vec!["line 6: fp-io (38651 lines) but crates/fp-io/src has 38651".to_owned()]
    );
    let unlisted = good.replace("(more than 35,000 lines)", "");
    assert_eq!(
        readme_number_violations(&unlisted, &tree),
        vec!["no \"(more than N lines)\" floor for crate fp-io".to_owned()]
    );

    let understated = good.replace("more than 8,500 `#[test]`", "more than 5,000 `#[test]`");
    assert_eq!(
        readme_number_violations(&understated, &tree),
        vec!["line 3: \"more than 5000 `#[test]` markers\" but the tree has 8978".to_owned()]
    );

    let inflated = good.replace("more than 600,000 lines", "more than 700,000 lines");
    assert_eq!(
        readme_number_violations(&inflated, &tree),
        vec![
            "line 4: \"more than 700000 lines of Rust under `crates/`\" but the tree has 651196"
                .to_owned()
        ]
    );

    // 8,900 is below the all-crates count but above the src count it names.
    let src_floor = good.replace(
        "more than 8,000 `#[test]` markers under",
        "more than 8,900 `#[test]` markers under",
    );
    assert_eq!(
        readme_number_violations(&src_floor, &tree),
        vec!["line 3: \"more than 8900 `#[test]` markers\" but the tree has 8214".to_owned()]
    );

    let bare = good.replace(
        "more than 550,000 lines of Rust under `src/`",
        "570,359 lines of Rust under `src/`",
    );
    assert_eq!(
        readme_number_violations(&bare, &tree),
        vec![
            "line 4: \"570359 lines of Rust under `src/`\" should be a \"more than N\" floor (the tree has 570359)"
                .to_owned()
        ]
    );

    let deleted = good.replace("1,387 packet JSON files spanning ", "");
    assert_eq!(
        readme_number_violations(&deleted, &tree),
        vec!["no packets claim found".to_owned()]
    );
}
