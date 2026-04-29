#![forbid(unsafe_code)]

use std::{fs, path::PathBuf, process::ExitCode};

use fp_conformance::{
    CiForensicsReport, CiGate, CiPipelineConfig, HarnessConfig, build_ci_forensics_report,
    run_ci_pipeline,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipelineKind {
    Commit,
    Full,
}

#[derive(Debug)]
struct CliArgs {
    pipeline: PipelineKind,
    gate: Option<CiGate>,
    fail_fast: bool,
    verify_sidecars: bool,
    json_out: Option<PathBuf>,
    python_bin: Option<String>,
    allow_system_pandas_fallback: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("fp-ci-gates error: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let args = parse_args()?;

    let mut harness = HarnessConfig::default_paths();
    harness.allow_system_pandas_fallback = args.allow_system_pandas_fallback;
    if let Some(python_bin) = args.python_bin {
        harness.python_bin = python_bin;
    }

    let gates = if let Some(gate) = args.gate {
        vec![gate]
    } else {
        match args.pipeline {
            PipelineKind::Commit => CiGate::commit_pipeline(),
            PipelineKind::Full => CiGate::pipeline(),
        }
    };

    let config = CiPipelineConfig {
        gates,
        fail_fast: args.fail_fast,
        harness_config: harness,
        verify_sidecars: args.verify_sidecars,
    };

    let result = run_ci_pipeline(&config);
    let forensics = build_ci_forensics_report(&result);

    println!("{result}");
    print_violation_summary(&forensics);

    if let Some(path) = args.json_out {
        write_json_report(&path, &forensics)?;
    }

    Ok(result.all_passed)
}

fn print_violation_summary(report: &CiForensicsReport) {
    if report.violations.is_empty() {
        return;
    }

    eprintln!("CI forensic violations:");
    for violation in &report.violations {
        eprintln!(
            "  {} ({}) failed: {}",
            violation.rule_id, violation.label, violation.summary
        );
        if !violation.errors.is_empty() {
            for error in &violation.errors {
                eprintln!("    - {error}");
            }
        }
        eprintln!("    repro_cmd: {}", violation.repro_cmd);
    }
}

fn write_json_report(
    path: &PathBuf,
    report: &CiForensicsReport,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let artifact = serde_json::json!({
        "bead_id": "bd-2gi.10",
        "artifact_path": path.display().to_string(),
        "report": report,
    });
    fs::write(path, serde_json::to_string_pretty(&artifact)?)?;
    println!("wrote ci_gate_forensics={}", path.display());
    Ok(())
}

fn parse_args() -> Result<CliArgs, Box<dyn std::error::Error>> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from<I>(args: I) -> Result<CliArgs, Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = String>,
{
    let mut pipeline = PipelineKind::Full;
    let mut gate = None;
    let mut fail_fast = true;
    let mut verify_sidecars = true;
    let mut json_out = None;
    let mut python_bin = None;
    let mut allow_system_pandas_fallback = false;

    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pipeline" => {
                let value = args.next().ok_or("--pipeline requires commit|full")?;
                pipeline = match value.as_str() {
                    "commit" => PipelineKind::Commit,
                    "full" => PipelineKind::Full,
                    _ => return Err(format!("unsupported pipeline: {value}").into()),
                };
            }
            "--gate" => {
                let value = args.next().ok_or("--gate requires a gate id")?;
                gate = Some(parse_gate(&value)?);
            }
            "--json-out" => {
                let value = args.next().ok_or("--json-out requires a file path")?;
                json_out = Some(PathBuf::from(value));
            }
            "--python-bin" => {
                let value = args.next().ok_or("--python-bin requires a path")?;
                python_bin = Some(value);
            }
            "--no-fail-fast" => {
                fail_fast = false;
            }
            "--no-verify-sidecars" => {
                verify_sidecars = false;
            }
            "--allow-system-pandas-fallback" => {
                allow_system_pandas_fallback = true;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(CliArgs {
        pipeline,
        gate,
        fail_fast,
        verify_sidecars,
        json_out,
        python_bin,
        allow_system_pandas_fallback,
    })
}

fn parse_gate(value: &str) -> Result<CiGate, Box<dyn std::error::Error>> {
    let gate = match value {
        "G1" | "G1Compile" => CiGate::G1Compile,
        "G2" | "G2Lint" => CiGate::G2Lint,
        "G3" | "G3Unit" => CiGate::G3Unit,
        "G4" | "G4Property" => CiGate::G4Property,
        "G4.5" | "G4_5Fuzz" => CiGate::G4_5Fuzz,
        "G5" | "G5Integration" => CiGate::G5Integration,
        "G6" | "G6Conformance" => CiGate::G6Conformance,
        "G7" | "G7Coverage" => CiGate::G7Coverage,
        "G8" | "G8E2e" => CiGate::G8E2e,
        _ => return Err(format!("unsupported gate: {value}").into()),
    };
    Ok(gate)
}

fn print_help() {
    println!(
        "fp-ci-gates\n\
         Usage:\n\
         \tfp-ci-gates [--pipeline commit|full] [--gate G6] [--json-out artifacts/ci/gate_forensics.json]\n\
         Options:\n\
         \t--pipeline <kind>   commit or full (default: full)\n\
         \t--gate <id>         run a single gate (G1..G8, e.g. G6 or G8E2e)\n\
         \t--json-out <path>   write machine-readable forensic report (JSON)\n\
         \t--python-bin <path> Python executable for live oracle runs (default: FP_PYTHON_BIN or python3)\n\
         \t--no-fail-fast      continue evaluating all configured gates after failures\n\
         \t--no-verify-sidecars  skip sidecar integrity check after gate success\n\
         \t--allow-system-pandas-fallback  allow non-legacy pandas import in live mode\n\
         \t-h, --help          show this help"
    );
}

#[cfg(test)]
mod tests {
    use super::{PipelineKind, parse_args_from};

    #[test]
    fn parse_args_accepts_python_bin_override() {
        let args = vec![
            "--pipeline".to_owned(),
            "commit".to_owned(),
            "--python-bin".to_owned(),
            "python3.12".to_owned(),
        ];

        let parsed = parse_args_from(args).expect("expected parse success");
        assert!(matches!(parsed.pipeline, PipelineKind::Commit));
        assert_eq!(parsed.python_bin.as_deref(), Some("python3.12"));
    }

    #[test]
    fn parse_args_rejects_missing_python_bin_value() {
        let args = vec!["--python-bin".to_owned()];
        let err = parse_args_from(args).expect_err("expected parse failure");
        assert!(
            err.to_string().contains("--python-bin requires a path"),
            "unexpected error: {err}"
        );
    }
}
