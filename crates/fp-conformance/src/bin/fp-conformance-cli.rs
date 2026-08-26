#![forbid(unsafe_code)]

use std::path::PathBuf;

use fp_conformance::{
    E2eConfig, FixtureGenerationRequest, HarnessConfig, NoopHooks, OracleMode, SuiteOptions,
    append_phase2c_drift_history, build_compat_closure_e2e_scenario_report,
    build_compat_closure_final_evidence_pack, enforce_packet_gates, generate_fixture_pilot_bundle,
    run_differential_by_id, run_e2e_suite, run_fault_injection_validation_by_id,
    run_packets_grouped, write_case_evidence_jsonl, write_compat_closure_e2e_scenario_report,
    write_compat_closure_final_evidence_pack, write_differential_validation_log,
    write_fault_injection_validation_report, write_grouped_artifacts,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw_args = std::env::args().collect::<Vec<_>>();
    let mut packet_filter: Option<String> = None;
    let mut oracle_mode = OracleMode::FixtureExpected;
    let mut python_bin: Option<String> = None;
    let mut write_artifacts = false;
    let mut require_green = false;
    let mut write_drift_history = false;
    let mut allow_system_pandas_fallback = false;
    let mut allow_fixture_fallback = false;
    let mut write_differential_validation = false;
    let mut write_fault_injection = false;
    let mut write_e2e_scenarios = false;
    let mut write_final_evidence_pack = false;
    let mut print_mismatches = false;
    let mut emit_evidence = false;
    let mut generate_fixture_pilot: Option<String> = None;
    let mut generated_packet_id: Option<String> = None;
    let mut generated_case_id: Option<String> = None;
    let mut generated_fixture_path: Option<PathBuf> = None;
    let mut generator_artifact_dir: Option<PathBuf> = None;
    let mut input_matrix = Vec::new();
    let mut intentional_divergence_notes = Vec::new();
    let mut repair_packets_per_block = 8_u32;

    let mut args = raw_args.iter().skip(1).cloned().peekable();
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
            "--allow-fixture-fallback" => {
                allow_fixture_fallback = true;
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
            "--emit-evidence" => {
                emit_evidence = true;
            }
            "--generate-fixture-pilot" => {
                let value = args
                    .next()
                    .ok_or("--generate-fixture-pilot requires a method family")?;
                generate_fixture_pilot = Some(value);
            }
            "--generated-packet-id" => {
                let value = args
                    .next()
                    .ok_or("--generated-packet-id requires a value")?;
                generated_packet_id = Some(value);
            }
            "--generated-case-id" => {
                let value = args.next().ok_or("--generated-case-id requires a value")?;
                generated_case_id = Some(value);
            }
            "--generated-fixture-path" => {
                let value = args
                    .next()
                    .ok_or("--generated-fixture-path requires a path")?;
                generated_fixture_path = Some(PathBuf::from(value));
            }
            "--generator-artifact-dir" => {
                let value = args
                    .next()
                    .ok_or("--generator-artifact-dir requires a path")?;
                generator_artifact_dir = Some(PathBuf::from(value));
            }
            "--input-matrix" => {
                let value = args.next().ok_or("--input-matrix requires a value")?;
                input_matrix.push(value);
            }
            "--intentional-divergence-note" => {
                let value = args
                    .next()
                    .ok_or("--intentional-divergence-note requires a value")?;
                intentional_divergence_notes.push(value);
            }
            "--repair-packets-per-block" => {
                let value = args
                    .next()
                    .ok_or("--repair-packets-per-block requires an integer")?;
                repair_packets_per_block = value.parse()?;
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
    config.allow_fixture_fallback = allow_fixture_fallback;
    if let Some(python_bin) = python_bin {
        config.python_bin = python_bin;
    }

    if let Some(method_family) = generate_fixture_pilot {
        let packet_id = generated_packet_id.unwrap_or_else(|| "FP-GEN-TN6QB2-001".to_owned());
        let case_id = generated_case_id
            .unwrap_or_else(|| "series_add_generated_alignment_strict_tn6qb2".to_owned());
        let fixture_path = generated_fixture_path.unwrap_or_else(|| {
            config
                .packet_fixture_root()
                .join(format!("fp_generated_tn6qb2_{case_id}.json"))
        });
        let artifact_dir = generator_artifact_dir.unwrap_or_else(|| {
            config
                .repo_root
                .join("artifacts/conformance")
                .join(format!("fixture-generator-tn6qb2-{packet_id}"))
        });
        let request = FixtureGenerationRequest {
            method_family,
            packet_id,
            case_id,
            output_fixture_path: fixture_path,
            artifact_dir,
            generation_command: raw_args.join(" "),
            input_matrix,
            intentional_divergence_notes,
            repair_packets_per_block,
        };
        let bundle = generate_fixture_pilot_bundle(&config, &request)?;
        println!(
            "generated packet={} case={} fixture={} manifest={} repair_symbols={} sidecar={} scrub={} decode_proof={}",
            bundle.packet_id,
            bundle.case_id,
            bundle.fixture_path.display(),
            bundle.manifest_path.display(),
            bundle.repair_symbol_manifest_path.display(),
            bundle.raptorq_sidecar_path.display(),
            bundle.integrity_scrub_path.display(),
            bundle.decode_proof_path.display()
        );
        return Ok(());
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
                let mismatch = case.mismatch.as_deref().unwrap_or("<no mismatch details>");
                let class = case.mismatch_class.as_deref().unwrap_or("<none>");
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

    if emit_evidence {
        for path in write_case_evidence_jsonl(&config, &reports)? {
            println!("wrote case_evidence={}", path.display());
        }
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
         \t--emit-evidence    Emit per-case machine-readable failure ledgers under artifacts/conformance/<packet>/<case>.jsonl\n\
         \t--write-e2e-scenarios Emit compat-closure E2E scenario matrix + replay bundle report\n\
         \t--write-final-evidence-pack Emit final compatibility evidence pack + migration + attestation bundle\n\
         \t--generate-fixture-pilot <family> Capture a deterministic pilot fixture with live pandas oracle output\n\
         \t--generated-packet-id <id> Packet id for --generate-fixture-pilot (default FP-GEN-TN6QB2-001)\n\
         \t--generated-case-id <id> Case id for --generate-fixture-pilot\n\
         \t--generated-fixture-path <path> Output fixture JSON path for --generate-fixture-pilot\n\
         \t--generator-artifact-dir <path> Output generator proof artifact directory\n\
         \t--input-matrix <entry> Extra input-matrix provenance entry; repeatable\n\
         \t--intentional-divergence-note <note> Divergence provenance note; repeatable\n\
         \t--repair-packets-per-block <n> RaptorQ repair packets per block for generated fixture bundle\n\
         \t--require-green      Fail with non-zero exit when any packet parity/gate check fails\n\
         \t--allow-system-pandas-fallback  Allow non-legacy pandas import when live oracle is enabled\n\
         \t-h, --help           Show this help"
    );
}
