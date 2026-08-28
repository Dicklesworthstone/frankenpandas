//! Report what a build worker can actually run, from INSIDE the worker.
//!
//! br-frankenpandas-l7r1p. Measuring FrankenPandas against the live incumbent in
//! one invocation on the worker requires the worker to HAVE the incumbent. That
//! is not a safe assumption: `a681970af` fixed 30 conformance tests that hard
//! failed on worker hz4 with `ModuleNotFoundError: No module named 'pandas'`
//! while passing on the submitting host.
//!
//! `rch exec` refuses non-compilation commands, so the probe has to arrive as
//! something cargo builds. Run it with:
//!
//!     rch exec -- cargo run --release --example worker_env_probe
//!
//! It prints the worker's hostname, its interpreter, whether pandas/numpy import
//! and at what version, and this executable's own sha256 — the same
//! self-reported ELF identity a head-to-head row would have to carry.

use std::process::Command;

fn sha256_of_self() -> String {
    // Hash our own image so a measurement can name the binary it came from,
    // exactly as the vs-pandas harness records `bench_elf_sha256`.
    match std::fs::read("/proc/self/exe") {
        Ok(bytes) => {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            format!("{:x}", hasher.finalize())
        }
        Err(err) => format!("<unreadable: {err}>"),
    }
}

fn probe(python: &str, code: &str) -> String {
    match Command::new(python).args(["-c", code]).output() {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err.lines().last().unwrap_or("").trim();
            format!("ERR({}): {last}", out.status)
        }
        Err(err) => format!("SPAWN-FAILED: {err}"),
    }
}

fn main() {
    println!("=== worker environment probe ===");
    println!(
        "hostname            {}",
        probe("hostname", "").replace("ERR", "n/a ERR")
    );
    println!("self_elf_sha256     {}", sha256_of_self());
    println!(
        "cwd                 {}",
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|e| e.to_string())
    );

    for python in ["python3", "python"] {
        println!("--- {python}");
        println!("  version           {}", probe(python, "import sys; print(sys.version.split()[0])"));
        println!("  executable        {}", probe(python, "import sys; print(sys.executable)"));
        println!("  pandas            {}", probe(python, "import pandas; print(pandas.__version__)"));
        println!("  numpy             {}", probe(python, "import numpy; print(numpy.__version__)"));
    }

    println!("=== verdict ===");
    let pandas = probe("python3", "import pandas; print(pandas.__version__)");
    if pandas.starts_with("ERR") || pandas.starts_with("SPAWN") {
        println!("INCUMBENT ABSENT — a same-invocation head-to-head vs live pandas");
        println!("CANNOT run on this worker. Any ratio produced here would be a");
        println!("FrankenPandas-vs-FrankenPandas self-comparison.");
    } else {
        println!("INCUMBENT PRESENT (pandas {pandas}) — head-to-head is possible here.");
    }
}
