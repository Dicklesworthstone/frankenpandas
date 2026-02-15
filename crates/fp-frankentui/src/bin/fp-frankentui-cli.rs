#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use fp_frankentui::{
    FsFtuiDataSource, FtuiDataSource, default_frankentui_e2e_scenarios,
    harness_config_from_repo_root, run_frankentui_e2e_matrix,
};

const DEFAULT_REPO_ROOT: &str = "../..";

#[derive(Debug, Clone)]
struct CliArgs {
    repo_root: PathBuf,
    packet: Option<String>,
    show_governance: bool,
    show_final_evidence: bool,
    forensic_log: Option<PathBuf>,
    run_e2e_matrix: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fp-frankentui-cli error: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let source = FsFtuiDataSource::from_repo_root(&args.repo_root);

    if let Some(packet_id) = args.packet.as_deref() {
        let packet = source
            .load_packet_snapshot(packet_id)
            .map_err(|error| error.to_string())?;
        println!(
            "packet={} gate_pass={} parity_green={} mismatches={} issues={}",
            packet.packet_id,
            packet.gate_result.as_ref().is_some_and(|gate| gate.pass),
            packet
                .parity_report
                .as_ref()
                .is_some_and(|report| report.is_green()),
            packet.mismatch_count.unwrap_or(0),
            packet.issues.len()
        );
    } else {
        let dashboard = source
            .load_conformance_dashboard()
            .map_err(|error| error.to_string())?;
        println!("{}", dashboard.render_plain());
    }

    if args.show_governance {
        match source
            .load_governance_gate_snapshot()
            .map_err(|error| error.to_string())?
        {
            Some(report) => println!(
                "governance path={} all_passed={} violations={} generated_unix_ms={}",
                report.path.display(),
                report.all_passed,
                report.violation_count,
                report.generated_unix_ms
            ),
            None => println!("governance report not found"),
        }
    }

    if args.show_final_evidence {
        let evidence = source
            .load_final_evidence_pack()
            .map_err(|error| error.to_string())?;
        println!("{}", evidence.render_plain());
    }

    if let Some(path) = args.forensic_log.as_deref() {
        let forensic = source
            .load_forensic_log(path)
            .map_err(|error| error.to_string())?;
        println!(
            "forensic path={} events={} malformed_lines={} missing={}",
            path.display(),
            forensic.events.len(),
            forensic.malformed_lines,
            forensic.missing
        );
    }

    if args.run_e2e_matrix {
        let harness = harness_config_from_repo_root(&args.repo_root);
        let scenarios = default_frankentui_e2e_scenarios();
        let reports = run_frankentui_e2e_matrix(&harness, &scenarios)
            .map_err(|error| format!("e2e matrix failed: {error}"))?;
        for scenario in reports {
            println!(
                "e2e scenario={} kind={} fixtures={} passed={} failed={} gates_pass={} replay_bundles={} forensic_failures={}",
                scenario.scenario.scenario_id,
                scenario.scenario.kind.as_str(),
                scenario.report.total_fixtures,
                scenario.report.total_passed,
                scenario.report.total_failed,
                scenario.report.gates_pass,
                scenario.replay_bundles.len(),
                scenario.forensics.failures.len()
            );
        }
    }

    Ok(())
}

fn parse_args() -> Result<CliArgs, String> {
    let mut repo_root = default_repo_root();
    let mut packet = None;
    let mut show_governance = false;
    let mut show_final_evidence = false;
    let mut forensic_log = None;
    let mut run_e2e_matrix = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repo-root" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--repo-root requires a path".to_owned())?;
                repo_root = PathBuf::from(value);
            }
            "--packet" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--packet requires a packet id".to_owned())?;
                packet = Some(value);
            }
            "--show-governance" => {
                show_governance = true;
            }
            "--show-final-evidence" => {
                show_final_evidence = true;
            }
            "--forensic-log" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--forensic-log requires a path".to_owned())?;
                forensic_log = Some(PathBuf::from(value));
            }
            "--run-e2e-matrix" => {
                run_e2e_matrix = true;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(CliArgs {
        repo_root,
        packet,
        show_governance,
        show_final_evidence,
        forensic_log,
        run_e2e_matrix,
    })
}

fn default_repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join(DEFAULT_REPO_ROOT)
}

fn print_help() {
    println!(
        "fp-frankentui-cli\n\
         Usage:\n\
         \tfp-frankentui-cli [--repo-root <path>] [--packet <FP-P2C-NNN>] [--show-governance] [--show-final-evidence] [--forensic-log <path>] [--run-e2e-matrix]\n\
         Options:\n\
         \t--repo-root <path>       repository root (default: crate root/{DEFAULT_REPO_ROOT})\n\
         \t--packet <packet_id>     show a single packet snapshot instead of full dashboard summary\n\
         \t--show-governance        include governance gate report summary (if present)\n\
         \t--show-final-evidence    include final evidence pack summary (parity + sidecar/decode integrity + risk notes)\n\
         \t--forensic-log <path>    parse forensic JSONL and print event/malformed counts\n\
         \t--run-e2e-matrix         execute FRANKENTUI E2E golden/regression/failure-injection scenario matrix\n\
         \t-h, --help               show this help"
    );
}
