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
is_frame = workload.startswith("df_transpose")
values = np.fromfile(path, dtype="<i8" if is_int else "<f8")
if is_frame:
    # Same shape as the corpus fixture: rows x 10, so the transposed frame has
    # one column per SOURCE ROW -- the shape the l4vzc claim is about.
    frame = pd.DataFrame(values.reshape(-1, 10))
else:
    series = pd.Series(values)
base = workload[:-6] if is_int else workload
# `_typed` only changes which FrankenPandas accessor the Rust arm uses; the
# incumbent's work is identical, so strip it here.
if base.endswith("_typed"):
    base = base[:-6]
sys.stderr.write("PANDAS_VERSION=%s\nNUMPY_VERSION=%s\n" % (pd.__version__, np.__version__))
sys.stderr.flush()
def run():
    # pandas' best available whole-frame materialisation, unchanged from the
    # banked lane: `.T` is a view over the block manager and `.to_numpy()` is
    # what forces it across the boundary.
    if base == "df_transpose_full_materialize": return frame.T.to_numpy().shape
    if base == "log":     return np.log(series)
    if base == "sqrt":    return np.sqrt(series)
    if base == "floor":   return np.floor(series)
    if base == "expm1":   return np.expm1(series)
    # br-frankenpandas-lrpp2: the eleven unary maps that have never been measured
    # and therefore inherit a `par_min` chosen for the cheapest ops in the family.
    # Every one of these is a numpy ufunc over the same (0, 1] fixture, so the
    # incumbent arm is doing exactly what the FrankenPandas arm is.
    if base == "exp":     return np.exp(series)
    if base == "log2":    return np.log2(series)
    if base == "cos":     return np.cos(series)
    if base == "tan":     return np.tan(series)
    if base == "asin":    return np.arcsin(series)
    if base == "acos":    return np.arccos(series)
    if base == "sinh":    return np.sinh(series)
    if base == "cosh":    return np.cosh(series)
    if base == "tanh":    return np.tanh(series)
    if base == "asinh":   return np.arcsinh(series)
    if base == "atanh":   return np.arctanh(series)
    # br-frankenpandas-lrpp2: fed the SHIFTED (1, 10] fixture, so this is the
    # kernel and not the out-of-domain path. See the note in `run_h2h`.
    if base == "acosh":   return np.arccosh(series)
    # Already opted in to `ELEMENTWISE_EXPENSIVE_PAR_MIN`, carried so the sweep has
    # in-run CONTROLS rather than comparing against numbers from another ELF.
    if base == "sin":     return np.sin(series)
    if base == "atan":    return np.arctan(series)
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
    _ = out if is_frame else out.iloc[0]
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

/// Number of source COLUMNS in the transpose fixture. The transposed frame then
/// has one column per source ROW, which is the shape l4vzc is about.
const TRANSPOSE_COLS: usize = 10;

/// Consecutive reps per arm slot. 1 reproduces the old rep-by-rep interleave.
const PHASE: usize = 8;

enum Subject {
    Col(fp_columnar::Column),
    Frame(fp_frame::DataFrame),
}

/// The transpose lane, matching the banked positional arm: transpose, then read
/// EVERY output column. `touched` and the `black_box` are load-bearing — without
/// them the loop is dead code and the lane would report an enormous false win.
/// `typed = false` reads each output column through `values()` (the `&[Scalar]`
/// boundary the banked lane uses); `typed = true` reads it through
/// `as_f64_slice()`, the zero-copy typed view a caller who wants NUMBERS would
/// actually use.
///
/// The pair is a DECOMPOSITION, not two competing lanes: both transpose the same
/// frame and touch every output column, so their difference is exactly what the
/// Scalar boundary costs on top of building the columns.
fn fp_transpose_rep_us(frame: &fp_frame::DataFrame, typed: bool) -> f64 {
    let start = Instant::now();
    let transposed = frame.transpose().expect("transpose");
    let mut touched = 0usize;
    for position in 0..transposed.num_columns() {
        let column = transposed
            .column_at(position)
            .expect("positional column present");
        touched += if typed {
            column.as_f64_slice().expect("typed f64 output column").len()
        } else {
            column.values().len()
        };
    }
    let elapsed = start.elapsed().as_nanos() as f64 / 1000.0;
    std::hint::black_box((&transposed, touched));
    elapsed
}

fn fp_one_rep_us(workload: &str, column: &fp_columnar::Column) -> f64 {
    let base = workload.strip_suffix("_int64").unwrap_or(workload);
    let start = Instant::now();
    let out = match base {
        "log" => column.log(),
        "sqrt" => column.sqrt(),
        "floor" => column.floor(),
        "expm1" => column.expm1(),
        // br-frankenpandas-lrpp2. `Column` has nineteen unary float maps; six carry
        // a per-op `par_min_override` and two more were measured and correctly left
        // on the shared default. These eleven have never been measured at ALL, so
        // they inherit `ELEMENTWISE_WITNESS_DEFAULT_PAR_MIN = 200_000` — a threshold
        // whose own source comment records per-element cost spanning ~80x across
        // this family, which is the argument that already produced the six overrides.
        //
        // Eleven lanes and not one representative, because unlike the earlier sweep
        // the question here is PER-OP: a representative can locate the crossover but
        // cannot tell you which side of it `cosh` falls on.
        //
        // ⚠ `acosh` is absent ON PURPOSE. Its domain is `x >= 1` and `fixture` is
        // (0, 1], so a lane for it would time the out-of-domain fallback rather than
        // the kernel. It needs its own fixture and is not in this sweep.
        "exp" => column.exp(),
        "log2" => column.log2(),
        "cos" => column.cos(),
        "tan" => column.tan(),
        "asin" => column.asin(),
        "acos" => column.acos(),
        "sinh" => column.sinh(),
        "cosh" => column.cosh(),
        "tanh" => column.tanh(),
        "asinh" => column.asinh(),
        "atanh" => column.atanh(),
        // br-frankenpandas-lrpp2. The last unmeasured op in the family. `run_h2h`
        // shifts its fixture into (1, 10] because `acosh` needs x >= 1; without
        // that this lane would time the missing-value fallback, not the kernel.
        "acosh" => column.acosh(),
        // CONTROLS, already on `ELEMENTWISE_EXPENSIVE_PAR_MIN`. The sweep's
        // break-even model is inherited from a table measured on a different ELF;
        // carrying the two ops that table opted in means each run can re-derive the
        // crossover from its OWN numbers instead of trusting a banked constant.
        "sin" => column.sin(),
        "atan" => column.atan(),
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
        // Three forms, so a single invocation can A/B whatever the question needs:
        //   `workload=NAME`  respawn on a DIFFERENT workload (same n/rounds)
        //   `KEY=VALUE`      respawn with that env set
        //   bare number      shorthand for FP_ELEMENTWISE_PAR_MIN
        let (key, value) = match par_min.split_once('=') {
            Some((key, value)) => (key.to_owned(), value.to_owned()),
            None => ("FP_ELEMENTWISE_PAR_MIN".to_owned(), par_min.clone()),
        };
        let respawn_workload = if key == "workload" {
            value.clone()
        } else {
            workload.clone()
        };
        let status = Command::new(exe)
            .args([&respawn_workload, &n.to_string(), &rounds.to_string()])
            .env(
                if key == "workload" { "FP_H2H_UNUSED" } else { &key },
                &value,
            )
            .stderr(Stdio::inherit())
            .stdout(Stdio::inherit())
            .status()
            .expect("respawn self with par_min");
        eprintln!("(respawn with {key}={value} exited {status})");
    }
}

fn run_h2h(workload: &str, n: usize, rounds: usize, label: &str) {
    let path = std::env::temp_dir().join(format!("fp_h2h_{workload}_{n}.bin"));
    let subject = if workload.starts_with("df_transpose") {
        let values = fixture(n * TRANSPOSE_COLS);
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&path, &bytes).expect("write fixture");
        // A RANGE index is required: the lazy transpose plan needs
        // `Index::int64_unit_range_labels`, and a Utf8 index silently falls back
        // to the eager materializer — a different code path from the banked lane.
        let index = fp_index::Index::from_range(0, n as i64, 1);
        let mut store: std::collections::BTreeMap<String, fp_columnar::Column> =
            std::collections::BTreeMap::new();
        for col in 0..TRANSPOSE_COLS {
            let slice: Vec<f64> = (0..n).map(|row| values[row * TRANSPOSE_COLS + col]).collect();
            store.insert(format!("{col}"), fp_columnar::Column::from_f64_values(slice));
        }
        let frame = fp_frame::DataFrame::new(index, store).expect("source frame");
        Subject::Frame(frame)
    } else if workload.ends_with("_int64") {
        let values = fixture_i64(n);
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&path, &bytes).expect("write fixture");
        Subject::Col(fp_columnar::Column::from_i64_values(values))
    } else {
        // br-frankenpandas-lrpp2. `acosh` is the ONE op in the 19-member unary
        // family that the sweep could not measure, and the reason is the fixture,
        // not the op: `fixture` lands in (0, 1] and `acosh` is defined on x >= 1,
        // so every value would be out of domain. A lane on the shared fixture
        // would have timed the missing-value fallback and reported it as the
        // kernel — a measurement of the wrong thing that would still have looked
        // like a number.
        //
        // Shifting into (1, 10] puts every element in domain and spans a decade,
        // so the lane exercises argument reduction rather than one narrow
        // magnitude. BOTH ARMS READ THIS SAME FILE, so the shift reaches numpy
        // and FrankenPandas identically and cannot become a difference between
        // the engines.
        let values: Vec<f64> = if workload == "acosh" {
            fixture(n).into_iter().map(|v| 1.0 + v * 9.0).collect()
        } else {
            fixture(n)
        };
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(&path, &bytes).expect("write fixture");
        Subject::Col(fp_columnar::Column::from_f64_values(values))
    };
    // One place decides which arm a workload runs, so the warm-up and every
    // timed round cannot disagree about it.
    let fp_rep = |subject: &Subject| -> f64 {
        match subject {
            Subject::Col(column) => fp_one_rep_us(workload, column),
            Subject::Frame(frame) => fp_transpose_rep_us(frame, workload.ends_with("_typed")),
        }
    };
    let mut incumbent = Incumbent::spawn(path.to_str().expect("utf8 path"), workload);

    // Warm both arms before any timed round.
    for _ in 0..3 {
        fp_rep(&subject);
        incumbent.one_rep_us();
    }

    let (mut fp_a, mut fp_b) = (Vec::new(), Vec::new());
    let (mut pd_a, mut pd_b) = (Vec::new(), Vec::new());
    for round in 0..rounds {
        // ROUND ORDER ALTERNATES ABBA / BAAB, and that is a correction to this
        // harness rather than a flourish. With ABBA alone, `pd_a` ALWAYS runs
        // immediately after a FrankenPandas rep and `pd_b` always after another
        // pandas rep — two structurally different positions, so the incumbent's
        // A/A null measures cache state rather than the incumbent.
        //
        // Invisible while the FP rep was ~600us; dominant on
        // `df_transpose_full_materialize @100k`, where the FP rep is ~40ms and
        // churns memory: the two pandas positions came out 454.5us vs 99.8us and
        // the pandas null hit 4.55 (limit 1.02), refusing the row. Alternating the
        // order gives each arm both positions equally.
        // PHASE-BLOCKED, not rep-by-rep, and the reason is measured. Alternating
        // the ORDER alone still left the incumbent's A/A null at 1.48-1.75 on
        // `df_transpose_full_materialize @100k` (limit 1.02), because pandas' rep
        // there is BIMODAL by cache state — ~100us warm against ~450us cold —
        // while FrankenPandas' rep is ~41ms and evicts everything between them.
        // A median over a 50/50 cold/warm mix is unstable no matter how the two
        // labels are balanced.
        //
        // Running PHASE consecutive reps per slot makes all but the first rep in
        // a phase warm, so each arm's median describes the arm rather than what
        // ran before it. Interleaving survives at phase granularity: the slots
        // still alternate ABBA/BAAB, so drift between phases cancels as before.
        let mut fp_phase = |out: &mut Vec<f64>| {
            for _ in 0..PHASE {
                out.push(fp_rep(&subject));
            }
        };
        let mut pd_phase = |out: &mut Vec<f64>, inc: &mut Incumbent| {
            for _ in 0..PHASE {
                out.push(inc.one_rep_us());
            }
        };
        if round % 2 == 0 {
            fp_phase(&mut fp_a);
            pd_phase(&mut pd_a, &mut incumbent);
            pd_phase(&mut pd_b, &mut incumbent);
            fp_phase(&mut fp_b);
        } else {
            pd_phase(&mut pd_a, &mut incumbent);
            fp_phase(&mut fp_a);
            fp_phase(&mut fp_b);
            pd_phase(&mut pd_b, &mut incumbent);
        }
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
