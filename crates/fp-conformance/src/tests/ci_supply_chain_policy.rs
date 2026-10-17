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
    /// Header of the generated `artifacts/perf/SCORECARD.md` (certified lanes,
    /// certified lanes still losing, overall certified geomean as printed).
    scorecard_lanes: usize,
    scorecard_losses: usize,
    scorecard_geomean: String,
}

impl TreeCounts {
    fn measure(root: &Path) -> Self {
        fn visit(dir: &Path, f: &mut impl FnMut(&Path)) {
            for entry in fs::read_dir(dir).expect("read directory") {
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
        let scorecard =
            fs::read_to_string(root.join("artifacts/perf/SCORECARD.md")).expect("read SCORECARD");
        // "- Certified lanes: **146** (...)": the bold value after `label`.
        let bold_after = |label: &str| -> String {
            scorecard
                .split_once(label)
                .and_then(|(_, rest)| rest.strip_prefix("**"))
                .and_then(|rest| rest.split_once("**"))
                .map(|(value, _)| value.to_owned())
                .expect("SCORECARD.md missing expected bold metric line")
        };
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
            scorecard_lanes: bold_after("Certified lanes: ")
                .parse()
                .expect("SCORECARD certified lanes"),
            scorecard_losses: bold_after("Certified lanes still losing: ")
                .parse()
                .expect("SCORECARD losses"),
            scorecard_geomean: bold_after("Overall certified geomean: ")
                .trim_end_matches('x')
                .to_owned(),
        };
        // Textual counts over every `.rs` file under crates/; lines are newline
        // counts, as `wc -l` reports them.
        let crates = root.join("crates");
        visit(&crates, &mut |p| {
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                return;
            }
            let bytes = fs::read(p).expect("read rust source file");
            let text = String::from_utf8_lossy(&bytes);
            let tests = text.matches("#[test]").count();
            let lines = text.matches('\n').count();
            counts.tests_all += tests;
            counts.rust_lines_all += lines;
            // Classify by the path below crates/, not the absolute path: a checkout that itself
            // lives under some `/src/` directory must not count every file as source (#45).
            let rel = p.strip_prefix(&crates).expect("under crates/");
            if rel.components().any(|c| c.as_os_str() == "src") {
                counts.tests_src += tests;
                counts.rust_lines_src += lines;
                counts.scope_sites += text.matches("thread::scope").count();
            }
            let mut parts = rel.iter();
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
    // Census numbers must be the generated scorecard's (the README said 143
    // lanes from a scorecard built on uncommitted, mtime-ordered rows).
    let (losing, mut lanes): (Vec<_>, Vec<_>) = number_claims(readme, " certified lanes")
        .into_iter()
        .partition(|claim| claim.tail.starts_with(" still losing"));
    lanes.extend(number_claims(readme, " certified benchmark lanes"));
    for (label, claims, want) in [
        ("certified lanes", lanes, tree.scorecard_lanes),
        (
            "certified lanes still losing",
            losing,
            tree.scorecard_losses,
        ),
    ] {
        if claims.is_empty() {
            out.push(format!("no {label} claim found"));
        }
        for claim in claims {
            if claim.value != want {
                out.push(format!(
                    "line {}: {} {label}, SCORECARD.md has {want}",
                    claim.line, claim.value
                ));
            }
        }
    }
    let geomean_prefix = "overall certified geomean of ";
    let geomeans: Vec<(usize, &str)> = readme
        .match_indices(geomean_prefix)
        .map(|(at, _)| {
            let rest = &readme[at + geomean_prefix.len()..];
            let value = &rest[..rest.find('x').unwrap_or(0)];
            (readme[..at].matches('\n').count() + 1, value)
        })
        .collect();
    if geomeans.is_empty() {
        out.push("no overall certified geomean claim found".to_owned());
    }
    for (line, value) in geomeans {
        if value != tree.scorecard_geomean {
            out.push(format!(
                "line {line}: geomean {value}x, SCORECARD.md has {}x",
                tree.scorecard_geomean
            ));
        }
    }
    // Rust_LOC shields.io badge floor ("Rust_LOC-750K%2B-orange.svg"): must be
    // a `K+` / `K%2B` floor within the 25% band of `tree.rust_lines_all`.
    for (at, _) in readme.match_indices("Rust_LOC-") {
        let line = readme[..at].matches('\n').count() + 1;
        let rest = &readme[at + "Rust_LOC-".len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let after = &rest[digits.len()..];
        if let Ok(k) = digits.parse::<usize>()
            && after.starts_with('K')
        {
            let floor_val = k * 1_000;
            let is_plus = after.starts_with("K%2B") || after.starts_with("K+");
            if !is_plus || !in_band(floor_val, tree.rust_lines_all) {
                out.push(format!(
                    "line {line}: Rust_LOC badge ({floor_val}{}) but crates/ has {}",
                    if is_plus { "+" } else { "" },
                    tree.rust_lines_all
                ));
            }
        }
    }
    // Per-crate floors in the Workspace Structure tree
    // ("│   ├── fp-io/  # ... (more than 35,000 lines)"); every crate needs one.
    // Prose headings of the form "`fp-frame` (a crate of more than N lines)"
    // are also checked against the same per-crate `src/` floor.
    let mut listed = BTreeSet::new();
    for (i, text) in readme.lines().enumerate() {
        if let Some((_, entry)) = text.split_once("── ") {
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
        } else if text.contains("(a crate of ") {
            for (name, &have) in &tree.crate_src_lines {
                if text.contains(name.as_str()) {
                    for claim in number_claims(text, " lines)") {
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
        tree.disc_total, 33,
        "DISCREPANCIES entry count changed; update README + this gate"
    );
    assert_eq!(
        tree.disc_active, 19,
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
        scorecard_lanes: 146,
        scorecard_losses: 10,
        scorecard_geomean: "3.970".to_owned(),
    };
    let good = "1,387 packet JSON files spanning 1,400 fixture files (147 occurrences across 8 files);\n\
        28 numbered divergence entries (16 active, the rest resolved);\n\
        more than 8,000 `#[test]` markers under `crates/*/src` and more than 8,500 `#[test]` markers across all of `crates/`;\n\
        more than 550,000 lines of Rust under `src/`; more than 600,000 lines of Rust under `crates/` (more than 550,000 of those lines live under `src/`).\n\
        │   ├── fp-frame/         # DataFrame, Series (more than 200,000 lines)\n\
        │   ├── fp-io/            # 14+ IO formats (more than 35,000 lines)\n\
        ├── artifacts/perf/       # Optimization round baselines (12 lines)\n\
        the scorecard reports 146 certified lanes, an overall certified geomean of 3.970x, and 10 certified lanes still losing.\n";
    assert_eq!(readme_number_violations(good, &tree), Vec::<String>::new());

    let stale_copy = format!("{good}Adding a new packet (1,341 packet files and counting).\n");
    assert_eq!(
        readme_number_violations(&stale_copy, &tree),
        vec!["line 9: 1341 packets, the tree has 1387".to_owned()]
    );

    // The 2026-09-02 census paragraph, left behind by a regenerated scorecard.
    let stale_census = good
        .replace("146 certified lanes", "143 certified lanes")
        .replace("geomean of 3.970x", "geomean of 4.000x");
    assert_eq!(
        readme_number_violations(&stale_census, &tree),
        vec![
            "line 8: 143 certified lanes, SCORECARD.md has 146".to_owned(),
            "line 8: geomean 4.000x, SCORECARD.md has 3.970x".to_owned(),
        ]
    );
    let stale_bench_lanes =
        format!("{good}faster than pandas across 143 certified benchmark lanes\n");
    assert_eq!(
        readme_number_violations(&stale_bench_lanes, &tree),
        vec!["line 9: 143 certified lanes, SCORECARD.md has 146".to_owned()]
    );
    let stale_losses = good.replace("10 certified lanes still", "14 certified lanes still");
    assert_eq!(
        readme_number_violations(&stale_losses, &tree),
        vec!["line 8: 14 certified lanes still losing, SCORECARD.md has 10".to_owned()]
    );
    let stale_badge =
        format!("{good}[![LOC](https://img.shields.io/badge/Rust_LOC-507K-orange.svg)]\n");
    assert_eq!(
        readme_number_violations(&stale_badge, &tree),
        vec!["line 9: Rust_LOC badge (507000) but crates/ has 651196".to_owned()]
    );
    let stale_prose_crate =
        format!("{good}## A Tour Through `fp-frame` (a crate of more than 150,000 lines)\n");
    assert_eq!(
        readme_number_violations(&stale_prose_crate, &tree),
        vec![
            "line 9: fp-frame (more than 150000 lines) but crates/fp-frame/src has 221529"
                .to_owned()
        ]
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

/// Structured README capability claim entry (`br-frankenpandas-rc0923-epic-truth-restoration-h5vo9.2`).
#[derive(Debug, Clone, Copy)]
struct ReadmeCapabilityClaim {
    id: &'static str,
    readme_snippet: &'static str,
    test_file: &'static str,
    test_name: &'static str,
    ci_job: &'static str,
}

const README_CLAIMS_REGISTRY: &[ReadmeCapabilityClaim] = &[
    ReadmeCapabilityClaim {
        id: "CLAIM-01-QUICK-EXAMPLE",
        readme_snippet: "## Quick Example",
        test_file: "crates/frankenpandas/tests/readme_quick_example.rs",
        test_name: "readme_quick_example_compiles_and_runs",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-02-LEAPFROG-JOIN",
        readme_snippet: "## Leapfrog Triejoin (AG-05) in Detail",
        test_file: "crates/fp-index/src/lib.rs",
        test_name: "leapfrog_union_three_indexes",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-03-GROUPBY-PATHS",
        readme_snippet: "### GroupBy: Three Execution Paths",
        test_file: "crates/fp-groupby/src/lib.rs",
        test_name: "dense_int64_groupby_matches_handcomputed_oracle_k3zcv",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-04-JOIN-TYPES",
        readme_snippet: "### Join Types",
        test_file: "crates/fp-join/src/lib.rs",
        test_name: "merge_asof_backward",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-05-EXPR-ENGINE",
        readme_snippet: "### Expression Engine",
        test_file: "crates/fp-expr/src/lib.rs",
        test_name: "evaluate_on_dataframe_matches_manual_context_eval",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-06-IO-POSTGRES",
        readme_snippet: "Pure synchronous Tokio-free PostgreSQL `PostgresConnection`",
        test_file: "crates/fp-io/src/lib.rs",
        test_name: "test_postgres_dtype_sql_mappings",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-07-IO-MYSQL",
        readme_snippet: "MySQL-backed `SqlConnection` impl",
        test_file: "crates/fp-io/src/lib.rs",
        test_name: "mysql_nullable_dtypes_declare_the_same_column_as_their_base",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-08-IO-SAS",
        readme_snippet: "`read_sas` handles sas7bdat and XPORT",
        test_file: "crates/fp-io/src/lib.rs",
        test_name: "read_sas_xport_builds_typed_frame_and_keeps_missing_numeric_as_null_9jvao",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-09-IO-CLIPBOARD",
        readme_snippet: "Rust `read_clipboard` / `to_clipboard` shell out to OS clipboard tools",
        test_file: "crates/fp-io/src/lib.rs",
        test_name: "clipboard_tsv_round_trips_through_read_clipboard_str",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-10-PLOTTING-SPECS",
        readme_snippet: "Returns backend-neutral `PlotSpec` / `BoxPlotSpec`",
        test_file: "crates/fp-frame/src/lib.rs",
        test_name: "dataframe_plotting_methods_return_specs",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-11-RUNTIME-BAYESIAN",
        readme_snippet: "### Bayesian Runtime Policy",
        test_file: "crates/fp-runtime/src/lib.rs",
        test_name: "conformal_guard_calibrates_after_sufficient_data",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-12-RUNTIME-RAPTORQ",
        readme_snippet: "**`RaptorQ envelope`** wraps durable artifacts",
        test_file: "crates/fp-runtime/src/lib.rs",
        test_name: "raptorq_envelope_from_source_bytes_invariants_b9vvk",
        ci_job: "test",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-13-CONFORMANCE-PACKETS",
        readme_snippet: "1,387 packet JSON files spanning 1,400 fixture files",
        test_file: "crates/fp-conformance/src/tests/live_oracle_dataframe_series_core.rs",
        test_name: "live_oracle_constructor_dtype_cases_match_pandas_bhyqp",
        ci_job: "conformance",
    },
    ReadmeCapabilityClaim {
        id: "CLAIM-14-PYTHON-BINDINGS",
        readme_snippet: "The PyO3 package (`crates/fp-python`) ships in-tree and builds from source via `maturin develop --release`",
        test_file: "crates/fp-python/src/lib.rs",
        test_name: "test_py_dataframe_indexers_and_dunders",
        ci_job: "linux",
    },
];

fn test_fn_status_in_source(source: &str, test_name: &str) -> Result<(), &'static str> {
    let needle = format!("fn {test_name}(");
    let Some(pos) = source.find(&needle) else {
        return Err("not_found");
    };
    // Inspect the immediately preceding attribute block above `fn <test_name>(`.
    let prefix = &source[..pos];
    for line in prefix.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if trimmed.starts_with("#[ignore") {
            return Err("ignored");
        }
        if !trimmed.starts_with("#[") && !trimmed.starts_with("//") {
            break;
        }
    }
    Ok(())
}

fn readme_capability_claim_violations(
    root: &Path,
    readme: &str,
    workflows: &[&str],
    claims: &[ReadmeCapabilityClaim],
) -> Vec<String> {
    let mut violations = Vec::new();
    for claim in claims {
        if !readme.contains(claim.readme_snippet) {
            violations.push(format!(
                "{}: README missing snippet {:?}",
                claim.id, claim.readme_snippet
            ));
        }
        let path = root.join(claim.test_file);
        match fs::read_to_string(&path) {
            Ok(source) => match test_fn_status_in_source(&source, claim.test_name) {
                Ok(()) => {}
                Err("not_found") => violations.push(format!(
                    "{}: test `{}` not found in {}",
                    claim.id, claim.test_name, claim.test_file
                )),
                Err("ignored") => violations.push(format!(
                    "{}: test `{}` in {} is #[ignore]d",
                    claim.id, claim.test_name, claim.test_file
                )),
                Err(other) => violations.push(format!(
                    "{}: unexpected status `{other}` for `{}` in {}",
                    claim.id, claim.test_name, claim.test_file
                )),
            },
            Err(_) => violations.push(format!(
                "{}: test file {} does not exist",
                claim.id, claim.test_file
            )),
        }
        let job_marker = format!("\n  {}:", claim.ci_job);
        if !workflows.iter().any(|wf| wf.contains(&job_marker)) {
            violations.push(format!(
                "{}: CI job `{}` not found in workflow definitions",
                claim.id, claim.ci_job
            ));
        }
    }
    violations
}

/// Every capability claim in `README_CLAIMS_REGISTRY` must appear in `README.md`,
/// map to a real, non-`#[ignore]`d test in the tree, and name a real CI job
/// (`br-frankenpandas-rc0923-epic-truth-restoration-h5vo9.2`).
#[test]
fn readme_capability_claims_registry_maps_to_real_unignored_tests_h5vo9_2() {
    let root = repo_root();
    let readme = fs::read_to_string(root.join("README.md")).expect("read README.md");
    let ci_yml = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci.yml");
    let wheels_yml =
        fs::read_to_string(root.join(".github/workflows/wheels.yml")).expect("read wheels.yml");
    let violations = readme_capability_claim_violations(
        &root,
        &readme,
        &[&ci_yml, &wheels_yml],
        README_CLAIMS_REGISTRY,
    );
    assert!(
        violations.is_empty(),
        "README capability claims registry violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn readme_capability_claims_registry_rejects_missing_or_ignored_probes_h5vo9_2() {
    let root = repo_root();
    let readme = "## Quick Example\n";
    let workflows = ["\n  test:\n"];
    let missing_test = [ReadmeCapabilityClaim {
        id: "CLAIM-MISSING-FN",
        readme_snippet: "## Quick Example",
        test_file: "crates/frankenpandas/tests/readme_quick_example.rs",
        test_name: "nonexistent_probe_fn_h5vo9_2",
        ci_job: "test",
    }];
    assert_eq!(
        readme_capability_claim_violations(&root, readme, &workflows, &missing_test),
        vec![
            "CLAIM-MISSING-FN: test `nonexistent_probe_fn_h5vo9_2` not found in crates/frankenpandas/tests/readme_quick_example.rs".to_owned()
        ]
    );

    let missing_snippet_and_job = [ReadmeCapabilityClaim {
        id: "CLAIM-DRIFT",
        readme_snippet: "## Vanished Section",
        test_file: "crates/frankenpandas/tests/readme_quick_example.rs",
        test_name: "readme_quick_example_compiles_and_runs",
        ci_job: "vanished-job",
    }];
    assert_eq!(
        readme_capability_claim_violations(&root, readme, &workflows, &missing_snippet_and_job),
        vec![
            "CLAIM-DRIFT: README missing snippet \"## Vanished Section\"".to_owned(),
            "CLAIM-DRIFT: CI job `vanished-job` not found in workflow definitions".to_owned(),
        ]
    );

    let ignored_src = "#[test]\n#[ignore = \"parked\"]\nfn parked_probe() {}\n";
    assert_eq!(
        test_fn_status_in_source(ignored_src, "parked_probe"),
        Err("ignored")
    );
    let active_src = "#[test]\nfn active_probe() {}\n";
    assert_eq!(test_fn_status_in_source(active_src, "active_probe"), Ok(()));
}

const REQUIRED_THREAT_MODEL_HEADINGS: &[&str] = &[
    "## 1. CSV Reader & Writer (`fp-io`)",
    "## 2. JSON & JSON-Lines Reader & Writer (`fp-io`)",
    "## 3. Excel Workbook Reader & Writer (`fp-io`)",
    "## 4. Parquet Columnar Reader & Writer (`fp-io`)",
    "## 5. Arrow IPC / Feather Reader & Writer (`fp-io`)",
    "## 6. XML Reader & Writer (`fp-io`)",
    "## 7. HTML Table Reader & Writer (`fp-io`)",
    "## 8. SAS (`SAS7BDAT` & `XPORT` v5) Reader (`fp-io`)",
    "## 9. Stata DTA Reader & Writer (`fp-io`)",
    "## 10. Pickle Snapshot Reader & Writer (`fp-io`)",
    "## 11. HDF5 Reader & Writer (`fp-io`)",
    "## 12. SQL Adapters & Identifier Quoting (`fp-io`)",
    "## 13. Expression Parser & Evaluator (`fp-expr`)",
    "## 14. Index Alignment & MultiIndex (`fp-index`)",
    "## 15. Columnar Storage & Vectorized Kernels (`fp-columnar` / `fp-frame` / `fp-groupby` / `fp-join`)",
    "## 16. Runtime Policy, Conformal Guard & RaptorQ Envelopes (`fp-runtime`)",
    "## 17. Python FFI Boundary (`fp-python`)",
    "## 18. Conformance Oracle Subprocess & Fixture Harness (`fp-conformance`)",
];

fn threat_model_violations(root: &Path, doc: &str, required_headings: &[&str]) -> Vec<String> {
    let mut violations = Vec::new();
    for &heading in required_headings {
        let Some(start) = doc.find(heading) else {
            violations.push(format!("missing subsystem heading: {heading}"));
            continue;
        };
        let rest = &doc[start + heading.len()..];
        let end = rest.find("\n## ").unwrap_or(rest.len());
        let section = &rest[..end];
        for required_field in [
            "- **Attack Surface:**",
            "- **Trust Boundary:**",
            "- **Mitigations (`file:line`):**",
            "- **Concrete Hostile Input & Verification Probe:**",
            "Hostile input:",
            "- **Known Gaps & Open Beads:**",
        ] {
            if !section.contains(required_field) {
                violations.push(format!("{heading}: missing `{required_field}`"));
            }
        }
        // Every section must cite at least one `crates/...::fn_name` or `fuzz/fuzz_targets/*.rs` probe
        // and every cited probe must exist on disk.
        let mut probe_count = 0usize;
        for token in section.split('`') {
            if let Some((rel, fn_name)) = token.split_once("::")
                && rel.starts_with("crates/")
                && rel.ends_with(".rs")
            {
                probe_count += 1;
                let path = root.join(rel);
                match fs::read_to_string(&path) {
                    Ok(source) => {
                        if test_fn_status_in_source(&source, fn_name) != Ok(()) {
                            violations.push(format!(
                                "{heading}: probe `{rel}::{fn_name}` missing or #[ignore]d"
                            ));
                        }
                    }
                    Err(_) => {
                        violations.push(format!("{heading}: probe file `{rel}` does not exist"))
                    }
                }
            } else if token.starts_with("fuzz/fuzz_targets/") && token.ends_with(".rs") {
                probe_count += 1;
                if !root.join(token).is_file() {
                    violations.push(format!("{heading}: fuzz target `{token}` does not exist"));
                }
            }
        }
        if probe_count == 0 {
            violations.push(format!(
                "{heading}: names no concrete `crates/...::fn` or `fuzz/fuzz_targets/*.rs` probe"
            ));
        }
    }
    violations
}

/// Per-subsystem threat model gate (`br-frankenpandas-rc0923-threat-model-per-subsystem-umlhr`).
#[test]
fn threat_model_covers_all_subsystems_with_real_probes_umlhr() {
    let root = repo_root();
    let doc = fs::read_to_string(root.join("docs/security/THREAT_MODEL.md"))
        .expect("read docs/security/THREAT_MODEL.md");
    let violations = threat_model_violations(&root, &doc, REQUIRED_THREAT_MODEL_HEADINGS);
    assert!(
        violations.is_empty(),
        "docs/security/THREAT_MODEL.md violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn threat_model_validator_rejects_missing_probes_or_sections_umlhr() {
    let root = repo_root();
    let incomplete_doc = "## 1. CSV Reader & Writer (`fp-io`)\n\
        - **Attack Surface:** csv\n\
        - **Trust Boundary:** bytes\n\
        - **Mitigations (`file:line`):** `crates/fp-io/src/lib.rs:2336`\n\
        - **Concrete Hostile Input & Verification Probe:**\n\
        - **Known Gaps & Open Beads:** none\n";
    assert_eq!(
        threat_model_violations(
            &root,
            incomplete_doc,
            &["## 1. CSV Reader & Writer (`fp-io`)"]
        ),
        vec![
            "## 1. CSV Reader & Writer (`fp-io`): missing `Hostile input:`".to_owned(),
            "## 1. CSV Reader & Writer (`fp-io`): names no concrete `crates/...::fn` or `fuzz/fuzz_targets/*.rs` probe".to_owned(),
        ]
    );
}
