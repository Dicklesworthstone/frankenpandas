#![forbid(unsafe_code)]

use fp_conformance::{
    E2eConfig, HarnessConfig, NoopHooks, OracleMode, SuiteOptions, append_phase2c_drift_history,
    build_compat_closure_e2e_scenario_report, build_compat_closure_final_evidence_pack,
    enforce_packet_gates, run_differential_by_id, run_e2e_suite,
    run_fault_injection_validation_by_id, run_packets_grouped,
    write_compat_closure_e2e_scenario_report, write_compat_closure_final_evidence_pack,
    write_differential_validation_log, write_fault_injection_validation_report,
    write_grouped_artifacts,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut packet_filter: Option<String> = None;
    let mut oracle_mode = OracleMode::FixtureExpected;
    let mut python_bin: Option<String> = None;
    let mut write_artifacts = false;
    let mut require_green = false;
    let mut write_drift_history = false;
    let mut allow_system_pandas_fallback = false;
    let mut write_differential_validation = false;
    let mut write_fault_injection = false;
    let mut write_e2e_scenarios = false;
    let mut write_final_evidence_pack = false;
    let mut print_mismatches = false;

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--packet-id" => {
                let value = args
                    .next()
                    .ok_or("--packet-id requires a value (e.g. FP-P2C-001)")?;
                packet_filter = Some(value);
            }
            "--oracle" => {
                let value = args.next().ok_or("--oracle requires fixture or live")?;
                oracle_mode = match value.as_str() {
                    "fixture" => OracleMode::FixtureExpected,
                    "live" => OracleMode::LiveLegacyPandas,
                    _ => return Err(format!("unsupported oracle mode: {value}").into()),
                };
            }
            "--python-bin" => {
                let value = args.next().ok_or("--python-bin requires a value")?;
                python_bin = Some(value);
            }
            "--write-artifacts" => {
                write_artifacts = true;
            }
            "--require-green" => {
                require_green = true;
            }
            "--write-drift-history" => {
                write_drift_history = true;
            }
            "--allow-system-pandas-fallback" => {
                allow_system_pandas_fallback = true;
            }
            "--write-differential-validation" => {
                write_differential_validation = true;
            }
            "--write-fault-injection" => {
                write_fault_injection = true;
            }
            "--write-e2e-scenarios" => {
                write_e2e_scenarios = true;
            }
            "--write-final-evidence-pack" => {
                write_final_evidence_pack = true;
            }
            "--print-mismatches" => {
                print_mismatches = true;
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => {
                return Err(format!("unknown argument: {other}").into());
            }
        }
    }

    let mut config = HarnessConfig::default_paths();
    config.allow_system_pandas_fallback = allow_system_pandas_fallback;
    if let Some(python_bin) = python_bin {
        config.python_bin = python_bin;
    }
    let options = SuiteOptions {
        packet_filter,
        oracle_mode,
    };

    let reports = run_packets_grouped(&config, &options)?;
    for report in &reports {
        println!(
            "packet={} suite={} fixtures={} passed={} failed={} green={}",
            report.packet_id.as_deref().unwrap_or("<all>"),
            report.suite,
            report.fixture_count,
            report.passed,
            report.failed,
            report.is_green()
        );
        if print_mismatches && report.failed > 0 {
            for case in &report.results {
                if case.status != fp_conformance::CaseStatus::Fail {
                    continue;
                }
                let mismatch = case
                    .mismatch
                    .as_deref()
                    .unwrap_or("<no mismatch details>");
                let class = case
                    .mismatch_class
                    .as_deref()
                    .unwrap_or("<none>");
                println!(
                    "  mismatch case={} op={:?} class={} details={}",
                    case.case_id, case.operation, class, mismatch
                );
            }
        }
    }

    if require_green {
        enforce_packet_gates(&config, &reports)?;
    }

    if write_artifacts {
        let written = write_grouped_artifacts(&config, &reports)?;
        for artifact in written {
            println!(
                "wrote packet={} parity={} sidecar={} decode_proof={} gate={} mismatch_corpus={}",
                artifact.packet_id,
                artifact.parity_report_path.display(),
                artifact.raptorq_sidecar_path.display(),
                artifact.decode_proof_path.display(),
                artifact.gate_result_path.display(),
                artifact.mismatch_corpus_path.display()
            );
        }
    }

    if write_artifacts || write_drift_history {
        let history_path = append_phase2c_drift_history(&config, &reports)?;
        println!("wrote drift_history={}", history_path.display());
    }

    if write_differential_validation {
        for report in &reports {
            let Some(packet_id) = report.packet_id.as_deref() else {
                continue;
            };
            let differential = run_differential_by_id(&config, packet_id, oracle_mode)?;
            let path = write_differential_validation_log(&config, &differential)?;
            println!(
                "wrote packet={} differential_validation_log={}",
                packet_id,
                path.display()
            );
        }
    }

    if write_fault_injection {
        for report in &reports {
            let Some(packet_id) = report.packet_id.as_deref() else {
                continue;
            };
            let validation = run_fault_injection_validation_by_id(&config, packet_id, oracle_mode)?;
            let path = write_fault_injection_validation_report(&config, &validation)?;
            println!(
                "wrote packet={} fault_injection_validation={}",
                packet_id,
                path.display()
            );
        }
    }

    if write_e2e_scenarios {
        let e2e_config = E2eConfig {
            harness: config.clone(),
            options: options.clone(),
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };
        let mut hooks = NoopHooks;
        let e2e_report = run_e2e_suite(&e2e_config, &mut hooks)?;
        let mut fault_reports = Vec::new();
        if let Some(packet_id) = options.packet_filter.as_deref() {
            fault_reports.push(run_fault_injection_validation_by_id(
                &config,
                packet_id,
                oracle_mode,
            )?);
        }
        let scenario_report = build_compat_closure_e2e_scenario_report(&e2e_report, &fault_reports);
        let path = write_compat_closure_e2e_scenario_report(&config.repo_root, &scenario_report)?;
        println!("wrote compat_closure_e2e_scenarios={}", path.display());
    }

    if write_final_evidence_pack {
        let mut differential_reports = Vec::new();
        let mut fault_reports = Vec::new();
        for report in &reports {
            let Some(packet_id) = report.packet_id.as_deref() else {
                continue;
            };
            differential_reports.push(run_differential_by_id(&config, packet_id, oracle_mode)?);
            fault_reports.push(run_fault_injection_validation_by_id(
                &config,
                packet_id,
                oracle_mode,
            )?);
        }

        let evidence_pack = build_compat_closure_final_evidence_pack(
            &config,
            &reports,
            &differential_reports,
            &fault_reports,
        )?;
        let paths = write_compat_closure_final_evidence_pack(&config, &evidence_pack)?;
        println!(
            "wrote compat_closure_final_evidence_pack={} migration_manifest={} attestation_summary={} all_checks_passed={} signature={}",
            paths.evidence_pack_path.display(),
            paths.migration_manifest_path.display(),
            paths.attestation_summary_path.display(),
            evidence_pack.all_checks_passed,
            evidence_pack.attestation_signature
        );
    }

    Ok(())
}

fn print_help() {
    println!(
        "fp-conformance-cli\n\
         Usage:\n\
         \tfp-conformance-cli [--packet-id FP-P2C-001] [--oracle fixture|live] [--write-artifacts] [--require-green]\n\
         Options:\n\
         \t--packet-id <id>     Run only one packet id\n\
         \t--oracle <mode>      fixture (default) or live\n\
         \t--python-bin <path>  Python executable for live oracle runs (default: FP_PYTHON_BIN or python3)\n\
         \t--write-artifacts    Emit parity + gate + RaptorQ sidecars per packet\n\
         \t--write-drift-history Append packet run summary to artifacts/phase2c/drift_history.jsonl\n\
         \t--write-differential-validation Emit differential validation JSONL per packet\n\
         \t--write-fault-injection Emit deterministic fault-injection validation report per packet\n\
         \t--write-e2e-scenarios Emit compat-closure E2E scenario matrix + replay bundle report\n\
         \t--write-final-evidence-pack Emit final compatibility evidence pack + migration + attestation bundle\n\
         \t--require-green      Fail with non-zero exit when any packet parity/gate check fails\n\
         \t--allow-system-pandas-fallback  Allow non-legacy pandas import when live oracle is enabled\n\
         \t-h, --help           Show this help"
    );
}
