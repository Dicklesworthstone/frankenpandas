//! Head-to-head against the LIVE incumbent, run entirely on one rch worker in a
//! single invocation.
//!
//!     rch exec -- cargo run --release --example h2h -- <workload> <n> <rounds>
//!
//! Everything a vs-incumbent row needs is produced here and printed to STDERR so
//! rch returns it: the ratio, both arms' medians, an A/A null per arm, this
//! executable's self-reported sha256, and the incumbent's self-reported version.
//!
//! ⚠️ SAME WORKER, SAME INVOCATION, SAME CLOCK — BUT NOT THE SAME PROCESS, and
//! the difference is worth stating rather than glossing. pandas is CPython; the
//! only way to run it inside this process is to link libpython, which is a C
//! dependency and forbidden here. So the incumbent arm is a LONG-LIVED CHILD
//! process driven one rep at a time over a pipe: spawned once, so interpreter
//! startup and pandas import are paid before timing begins and cannot leak into
//! any rep. Interleaving is therefore real at the round level, which is what the
//! balanced square needs.
//!
//! BOTH ARMS SEE BYTE-IDENTICAL INPUT. The fixture is generated once in Rust and
//! handed to the child as raw little-endian f64 through a file, read back with
//! `numpy.fromfile`. Generating the same sequence twice from a shared formula
//! would let a divergence hide as a "measurement".
//!
//! ORDER IS ABBA per round pair, so a monotone drift in machine speed cancels to
//! first order instead of being attributed to whichever arm ran first.

use sha2::Digest as _;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

fn self_sha256() -> String {
    let Ok(path) = std::env::current_exe() else {
        return "unavailable".to_owned();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return "unavailable".to_owned();
    };
    use std::fmt::Write as _;
    let digest = sha2::Sha256::digest(&bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!("{hex} ({} bytes)", bytes.len())
}

/// Deterministic fixture: a plain LCG so a rerun at the same `n` is byte-identical.
/// Values land in (0, 1] because `log` needs a positive domain.
fn fixture(n: usize) -> Vec<f64> {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    (0..n)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 11) as f64) / ((1_u64 << 53) as f64) + f64::MIN_POSITIVE
        })
        .collect()
}

/// Int64 sibling of [`fixture`]. Values are positive so `log`/`sqrt` stay in
/// domain, and small enough that `x as f64` is exact (|x| < 2^53), so the
/// widening cannot itself be the difference between the arms.
fn fixture_i64(n: usize) -> Vec<i64> {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    (0..n)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 24) as i64) + 1
        })
        .collect()
}

const DRIVER: &str = r#"
import sys, time, numpy as np, pandas as pd
path, workload = sys.argv[1], sys.argv[2]
# An `_int64` workload feeds BOTH arms an integer column, so the incumbent pays
# its own widening too and the comparison is not float-vs-int.
is_int = workload.endswith("_int64")
values = np.fromfile(path, dtype="<i8" if is_int else "<f8")
series = pd.Series(values)
base = workload[:-6] if is_int else workload
sys.stderr.write("PANDAS_VERSION=%s\nNUMPY_VERSION=%s\n" % (pd.__version__, np.__version__))
sys.stderr.flush()
def run():
    if base == "log":     return np.log(series)
    if base == "sqrt":    return np.sqrt(series)
    if base == "floor":   return np.floor(series)
    if base == "expm1":   return np.expm1(series)
    raise SystemExit("unknown workload " + workload)
run()  # warm the ufunc dispatch before any timed rep
print("READY", flush=True)
for line in sys.stdin:
    if line.strip() != "RUN":
        break
    t0 = time.perf_counter_ns()
    out = run()
    t1 = time.perf_counter_ns()
    # Touch the result so a lazy/deferred pandas cannot make the rep vacuous.
    _ = out.iloc[0]
    print((t1 - t0) / 1000.0, flush=True)
"#;

struct Incumbent {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Incumbent {
    fn spawn(path: &str, workload: &str) -> Self {
        let mut child = Command::new("python3")
            .args(["-u", "-c", DRIVER, path, workload])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn python3 incumbent");
        let stdin = child.stdin.take().expect("child stdin");
        let mut stdout = BufReader::new(child.stdout.take().expect("child stdout"));
        let mut ready = String::new();
        stdout.read_line(&mut ready).expect("incumbent READY");
        assert!(
            ready.trim() == "READY",
            "incumbent did not start cleanly: {ready:?}"
        );
        Self { child, stdin, stdout }
    }

    fn one_rep_us(&mut self) -> f64 {
        writeln!(self.stdin, "RUN").expect("drive incumbent");
        self.stdin.flush().expect("flush");
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("incumbent timing");
        line.trim().parse().expect("incumbent timing is a number")
    }
}

fn fp_one_rep_us(workload: &str, column: &fp_columnar::Column) -> f64 {
    let base = workload.strip_suffix("_int64").unwrap_or(workload);
    let start = Instant::now();
    let out = match base {
        "log" => column.log(),
        "sqrt" => column.sqrt(),
        "floor" => column.floor(),
        "expm1" => column.expm1(),
        other => panic!("unknown workload {other}"),
    }
    .expect("frankenpandas arm");
    let elapsed = start.elapsed().as_nanos() as f64 / 1000.0;
    std::hint::black_box(&out);
    elapsed
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let workload = args.get(1).cloned().unwrap_or_else(|| "log".to_owned());
    let n: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(100_000);
    let rounds: usize = args.get(3).and_then(|a| a.parse().ok()).unwrap_or(64);
    // Optional 4th arg: an FP_ELEMENTWISE_PAR_MIN to A/B against. The parent
    // measures itself first, then RESPAWNS ITSELF with that env set, so both
    // FrankenPandas configurations are measured against the live incumbent
    // inside ONE rch invocation on ONE worker.
    //
    // Spawned rather than `std::env::set_var`: setting a process's own
    // environment is `unsafe` in edition 2024 and this workspace forbids unsafe.
    // A child's environment is safe to set, and it also guarantees the gate is
    // read fresh rather than after some other code already cached it.
    let par_min = args.get(4).cloned();

    run_h2h(&workload, n, rounds, "DEFAULT");

    if let Some(par_min) = par_min
        && std::env::var("FP_ELEMENTWISE_PAR_MIN").is_err()
    {
        let exe = std::env::current_exe().expect("current exe");
        // Accept either a bare par_min value (back-compatible) or an explicit
        // KEY=VALUE, so any env-gated constant can be A/B'd in one invocation.
        let (key, value) = match par_min.split_once('=') {
            Some((key, value)) => (key.to_owned(), value.to_owned()),
            None => ("FP_ELEMENTWISE_PAR_MIN".to_owned(), par_min.clone()),
        };
        let status = Command::new(exe)
            .args([&workload, &n.to_string(), &rounds.to_string()])
            .env(&key, &value)
            .stderr(Stdio::inherit())
            .stdout(Stdio::inherit())
            .status()
            .expect("respawn self with par_min");
        eprintln!("(respawn with {key}={value} exited {status})");
    }
}

fn run_h2h(workload: &str, n: usize, rounds: usize, label: &str) {
    let path = std::env::temp_dir().join(format!("fp_h2h_{workload}_{n}.bin"));
    let column = if workload.ends_with("_int64") {
        let values = fixture_i64(n);
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&path, &bytes).expect("write fixture");
        fp_columnar::Column::from_i64_values(values)
    } else {
        let values = fixture(n);
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&path, &bytes).expect("write fixture");
        fp_columnar::Column::from_f64_values(values)
    };
    let mut incumbent = Incumbent::spawn(path.to_str().expect("utf8 path"), workload);

    // Warm both arms before any timed round.
    for _ in 0..3 {
        fp_one_rep_us(workload, &column);
        incumbent.one_rep_us();
    }

    let (mut fp_a, mut fp_b) = (Vec::new(), Vec::new());
    let (mut pd_a, mut pd_b) = (Vec::new(), Vec::new());
    for _ in 0..rounds {
        // A B B A — the second half mirrors the first so drift cancels.
        fp_a.push(fp_one_rep_us(workload, &column));
        pd_a.push(incumbent.one_rep_us());
        pd_b.push(incumbent.one_rep_us());
        fp_b.push(fp_one_rep_us(workload, &column));
    }

    let fp_med_a = median(&mut fp_a.clone());
    let fp_med_b = median(&mut fp_b.clone());
    let pd_med_a = median(&mut pd_a.clone());
    let pd_med_b = median(&mut pd_b.clone());
    let mut fp_all: Vec<f64> = fp_a.iter().chain(fp_b.iter()).copied().collect();
    let mut pd_all: Vec<f64> = pd_a.iter().chain(pd_b.iter()).copied().collect();
    let fp_med = median(&mut fp_all);
    let pd_med = median(&mut pd_all);

    // A/A null: the same engine against itself across the two interleaved
    // positions. It must sit near 1.0 or the run says nothing about the effect.
    let fp_null = fp_med_a / fp_med_b;
    let pd_null = pd_med_a / pd_med_b;
    let ratio = pd_med / fp_med; // > 1.0 means FrankenPandas is FASTER
    let null_margin = (fp_null - 1.0).abs().max((pd_null - 1.0).abs());
    let effect = (ratio - 1.0).abs();

    let verdict = if null_margin > 0.02 {
        "NULL_UNDECIDABLE"
    } else if effect < 2.0 * null_margin {
        "UNDECIDABLE"
    } else if ratio > 1.0 {
        "FASTER"
    } else {
        "SLOWER"
    };

    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    eprintln!("=== H2H {workload} n={n} rounds={rounds} arm={label} par_min_env={} two_pass_env={} ===",
        std::env::var("FP_ELEMENTWISE_PAR_MIN").unwrap_or_else(|_| "<unset>".to_owned()),
        std::env::var("FP_INT64_WIDEN_TWO_PASS").unwrap_or_else(|_| "<unset>".to_owned()));
    eprintln!("fp_elf_sha256   {}", self_sha256());
    eprintln!("host            {}", std::fs::read_to_string("/etc/hostname").unwrap_or_default().trim());
    eprintln!("loadavg         {}", load.trim());
    eprintln!("fp_median_us    {fp_med:.3}   (arm_a {fp_med_a:.3}  arm_b {fp_med_b:.3})");
    eprintln!("pd_median_us    {pd_med:.3}   (arm_a {pd_med_a:.3}  arm_b {pd_med_b:.3})");
    eprintln!("aa_null_fp      {fp_null:.5}");
    eprintln!("aa_null_pandas  {pd_null:.5}");
    eprintln!("null_margin     {null_margin:.5}   effect {effect:.5}");
    eprintln!("RATIO           {ratio:.4}  ({verdict})   >1 = FrankenPandas faster");

    let _ = incumbent.stdin.write_all(b"STOP\n");
    let _ = incumbent.child.wait();
}
