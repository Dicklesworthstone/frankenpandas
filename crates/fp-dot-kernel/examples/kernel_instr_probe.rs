//! Instruction-count probe for the blocked f64 GEMM, isolated from fp-bench.
//!
//! br-frankenpandas-633fb. Whole-bench differencing put the kernel at 1.866
//! instructions per FMA against a 0.554 reference, but could not say WHERE the
//! 3.4x goes: the bench number also carries `dot()` plumbing, per-call output
//! allocation, packing, and the `dim = 316` tails, and guessing between those is
//! how a lever gets attributed to the wrong mechanism.
//!
//! Usage: `kernel_instr_probe <dim> <reps> <mode>` with mode one of
//! `prepacked` | `block` | `axpy`. Run it TWICE, at `reps = 0` and `reps = N`,
//! and subtract: everything outside the timed loop -- process start, fixture
//! build, allocation of the inputs -- is identical between the two, so the
//! difference is exactly `N` kernel invocations. That is the same null
//! subtraction that fixed the numpy measurement; a two-SIZE difference cannot be
//! used here because startup is not constant across shapes.
//!
//! Instructions per FMA = delta / (reps * dim^3).

use std::hint::black_box;

fn build(dim: usize) -> Vec<Vec<f64>> {
    // Deterministic, no rand: a cheap LCG so two runs at the same dim build
    // byte-identical inputs and the subtraction is exact.
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    (0..dim)
        .map(|_| {
            (0..dim)
                .map(|_| {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    ((state >> 11) as f64) / ((1_u64 << 53) as f64)
                })
                .collect()
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dim: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(316);
    let reps: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(0);
    let mode: &str = args.get(3).map_or("prepacked", |s| s.as_str());

    let a_data = build(dim);
    let b_data = build(dim);
    let a_slices: Vec<&[f64]> = a_data.iter().map(|c| c.as_slice()).collect();
    let b_cols: Vec<&[f64]> = b_data.iter().map(|c| c.as_slice()).collect();

    // Packed ONCE, outside the loop, so `prepacked` measures the microkernel
    // alone and `block` measures microkernel + packing. The difference between
    // the two modes is the packing cost.
    let packed = fp_dot_kernel::pack_a_panel(&a_slices, dim);

    let mut checksum = 0.0_f64;
    for _ in 0..reps {
        match mode {
            "block" => {
                let out = fp_dot_kernel::materialize_float64_dot_block(&a_slices, &b_cols, dim);
                checksum += black_box(&out)[0][0];
            }
            "axpy" => {
                // The per-column kernel the blocked path replaces: one pass over
                // the whole A panel per output column.
                for b_col in &b_cols {
                    let out = fp_dot_kernel::materialize_float64_dot(&a_slices, b_col, dim);
                    checksum += black_box(&out)[0];
                }
            }
            _ => {
                let out = fp_dot_kernel::materialize_float64_dot_block_prepacked(
                    &packed, &a_slices, &b_cols, dim,
                );
                checksum += black_box(&out)[0][0];
            }
        }
    }
    // stderr so stdout stays clean for the caller; also keeps the loop live.
    eprintln!("dim={dim} reps={reps} mode={mode} checksum={checksum}");
}
