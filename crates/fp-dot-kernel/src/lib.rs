#![feature(portable_simd)]
//! ISA-isolated `f64` dot-product materialization.
//!
//! br-frankenpandas-oxv4u. This crate exists for ONE reason: it is the only
//! member of the workspace compiled with `+avx2,+fma`, via
//! `[profile.<p>.package.fp-dot-kernel] rustflags` in the workspace manifest.
//! Everything else stays at the baseline target, because a blanket
//! `x86-64-v3` policy was MEASURED and REJECTED (2026-07-31 CyanLynx: `sqrt`
//! 0.361x, `log` regressed). Keeping the flag to one crate is what makes the
//! win available without that collateral — `sqrt`/`log` live in fp-columnar and
//! are untouched by this profile entry.
//!
//! ⚠️ THREE OTHER MECHANISMS WERE TRIED AND ARE DEAD (compiled, not argued —
//! docs/NEGATIVE_EVIDENCE.md 2026-08-16 SilverFalcon):
//!   * `#[target_feature]` on a safe fn + `is_x86_feature_detected` guard →
//!     `error[E0133]`, the CALL still requires `unsafe`.
//!   * a narrow `#[allow(unsafe_code)]` on one module → `error[E0453]`,
//!     `forbid` cannot be relaxed locally.
//!   * `std::simd` portable SIMD on a baseline build → compiles, but emits
//!     0 ymm / 6 mulpd / 24 addpd: 4-wide vectors lower to 2×SSE2. No ISA gain.
//!
//! Per-crate rustflags is the only one that produced ymm registers, and it does
//! so with NO `unsafe` keyword anywhere — this crate keeps `forbid(unsafe_code)`.
//!
//! ⚠️ CALLERS MUST GUARD. Nothing here is safe to enter on a pre-AVX2 CPU: the
//! compiler is told the features exist, so it emits them unconditionally and the
//! process takes SIGILL rather than a graceful error. The guard is
//! `is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma")` at the
//! call site, in a BASELINE crate, with the original kernel as the else arm.
//! That is discipline, not types — this crate cannot enforce it.

#![forbid(unsafe_code)]

use std::simd::{
    Mask, Simd, StdFloat as _,
    cmp::{SimdPartialEq, SimdPartialOrd},
    num::SimdFloat as _,
};

/// Materialize `out[row] = Σ_j a_slices[j][row] * b_col[j]` for `len` rows.
///
/// AXPY loop order: outer over the `k` A-columns, inner streaming over the
/// `len` output rows. The prior DOT order (outer row, inner column) read a
/// strided, per-access-bounds-checked hop across `k` separate allocations —
/// one cache miss per (row, col), 0.59 GFLOP/s for a 1000×1000 materialize.
/// This order hands the inner loop a contiguous unit-stride slice.
///
/// BIT-IDENTICAL to the fp-columnar kernel it mirrors: each `out[row]` still
/// accumulates `j = 0..k` in the same order, so the non-associative f64
/// addition sequence is unchanged. AVX2 widens the lanes; it does not reorder
/// the sum. Note that Rust does not contract `a*b+c` into an FMA without
/// fast-math, so `+fma` changes the available registers, not the arithmetic —
/// which is why an `+avx2,+fma` build of the identical source previously came
/// out BIT-IDENTICAL to the baseline one (both ELFs checksummed 4957dea0fe3e2ed1).
///
/// ⚠️ `#[inline(never)]` AND NON-GENERIC, both load-bearing. MEASURED TRAP on
/// br-frankenpandas-oxv4u: an `#[inline]` or generic entry point codegens in the
/// CALLER's crate at baseline ISA and the flag is silently lost — 0 ymm, 2 mulpd,
/// a green build with correct values and no speedup, invisible without
/// disassembly. Do not add generics or relax the inline attribute here.
#[inline(never)]
#[must_use]
pub fn materialize_float64_dot(a_slices: &[&[f64]], b_col: &[f64], len: usize) -> Vec<f64> {
    debug_assert_eq!(a_slices.len(), b_col.len());
    let mut out = vec![0.0_f64; len];
    for (column, &scale) in a_slices.iter().zip(b_col.iter()) {
        let column = &column[..len];
        for (slot, &value) in out.iter_mut().zip(column.iter()) {
            *slot += value * scale;
        }
    }
    out
}

/// Output rows held in vector registers by the blocked kernel: 8 `f64` = 2 ymm.
const MR: usize = 8;

/// Output COLUMNS computed per pass of the A panel by the blocked kernel.
///
/// The register budget is what picks this. `MR / 4 * NR` ymm accumulators must
/// stay live across the whole `k` loop alongside two A vectors and a broadcast
/// scalar, so `8x6` uses 12 of 16 ymm. MEASURED on this host, `dim = 1000`,
/// one +avx2 binary, interleaved, best-of-5, against the AXPY kernel above:
///
/// | tile | 4x4  | 4x8  | 4x12 | 8x4  | 8x6  | 8x8  | 16x2 | 16x4 |
/// |------|------|------|------|------|------|------|------|------|
/// | ratio|1.038x|1.104x|0.771x|1.663x|1.710x|1.345x|0.970x|1.270x|
///
/// The shape either side of 8x6 loses, in both directions, which is the
/// signature of a register-pressure optimum rather than a fitted constant.
pub const DOT_BLOCK_COLUMNS: usize = 6;

/// Register-blocked microkernel over a PACKED A row-panel.
///
/// `packed[p * MR + i]` is `A[row + i][p]`, so the `k` loop streams contiguous
/// memory. Without packing this same tile shape measured 0.597x — SLOWER than
/// the AXPY kernel — because each `p` hopped `len * 8` bytes to the next A
/// column and took a cache miss per (row-block, p). The packing IS the lever;
/// the register tile alone is not.
#[inline(always)]
fn micro_kernel_packed(
    packed: &[f64],
    b_cols: &[&[f64]],
    k: usize,
    row: usize,
    out: &mut [Vec<f64>],
    j0: usize,
    nr: usize,
) {
    let mut acc = [[0.0_f64; MR]; DOT_BLOCK_COLUMNS];
    for p in 0..k {
        let Ok(a_tile) = <&[f64; MR]>::try_from(&packed[p * MR..p * MR + MR]) else {
            return;
        };
        for (j, dst) in acc.iter_mut().enumerate().take(nr) {
            let scale = b_cols[j0 + j][p];
            for i in 0..MR {
                dst[i] += a_tile[i] * scale;
            }
        }
    }
    for (j, tile) in acc.iter().enumerate().take(nr) {
        out[j0 + j][row..row + MR].copy_from_slice(tile);
    }
}

/// Materialize SEVERAL output columns of one `df.dot` in a single pass over the
/// shared A panel: `out[j][row] = Σ_p a_slices[p][row] * b_cols[j][p]`.
///
/// BIT-IDENTICAL to calling [`materialize_float64_dot`] once per column, and by
/// construction rather than by measurement: every `out[j][row]` still
/// accumulates `p = 0..k` in ascending order with a separate `mul` and `add`
/// (no `mul_add`, no reassociation, no k-splitting), so the non-associative f64
/// sequence is the same one the per-column kernel produces. The probe that
/// developed this compared all 1_000_000 outputs of both kernels bit-for-bit at
/// every tile shape in the table above and found ZERO differing elements.
///
/// WHY IT IS FASTER, and it is not parallelism — this runs on one thread. The
/// per-column kernel re-streams the entire A panel for EVERY output column:
/// `n` passes over `m * k * 8` bytes. This one keeps an `MR x NR` output tile in
/// vector registers across the whole `k` loop, so A is streamed `n / NR` times
/// and the output tile costs no memory traffic at all inside the loop. Measured
/// 16.79 -> 28.53 GFLOP/s at `dim = 1000`, which is ~89% of this machine's
/// AVX2-without-FMA ceiling (4 lanes x (1 mul + 1 add) per cycle) — the
/// remaining headroom needs `mul_add`, and that WOULD change the bits.
///
/// ⚠️ 2D CACHE BLOCKING (MC x KC) WAS BUILT AND REJECTED — 2026-08-27. The
/// obvious next move is BLIS-style blocking: sweep a `KC`-deep block of `k`
/// across a group of `MC` rows so the A working set is bounded by `KC` instead
/// of by `k`. It was implemented (KC=256, MC=8 row blocks, a `_accum`
/// microkernel that loads the tile's running sum, continues it, and stores back
/// — bit-identical, because the `k` blocks are visited in ASCENDING order and no
/// partial sum is ever rounded separately and recombined) and it LOSES:
///
///   * serial, `dim = 1000`, W=1: 75132/76418us -> 81663/80721us, ~8% SLOWER,
///     consistent across passes.
///   * certified vs live pandas @100k, pinned incumbent, clean A/A nulls:
///     FP p50 1141.8us -> 1175.8us, ratio 0.519 -> 0.521, i.e. unchanged.
///
/// The reason it cannot help was visible BEFORE the build and is worth keeping:
/// `perf stat` on this kernel gives IPC 5.32 serial at a 5.38% cache-miss rate,
/// and 2.91 at 63 threads at 2.86%. It is ISSUE-bound, not memory-bound, so
/// blocking has nothing to recover and the extra per-`k`-block load/store of the
/// output tile is pure cost. A COLUMN-panel reorder (B panel hoisted out of the
/// row loop) was rejected the same day for the same reason: 11-18% slower at 1M.
///
/// Do not re-open either without first showing a cache-miss rate that makes the
/// kernel memory-bound.
///
/// br-frankenpandas-mti15. The 2026-07-23 ledger closed this vein with an
/// explicit retry predicate — "re-open df_dot only for a hand-written GEMM
/// microkernel (register-blocked, packed panels)" — and recorded the blocker as
/// the SSE2 build ISA. This crate is that opening: it is the one workspace
/// member compiled `+avx2,+fma`.
///
/// ⚠️ `#[inline(never)]` and non-generic for the same reason as its sibling: an
/// inlinable or generic entry point codegens in the CALLER's baseline crate and
/// the ISA is silently lost.
#[inline(never)]
#[must_use]
pub fn materialize_float64_dot_block(
    a_slices: &[&[f64]],
    b_cols: &[&[f64]],
    len: usize,
) -> Vec<Vec<f64>> {
    let packed = pack_a_panel(a_slices, len);
    materialize_float64_dot_block_prepacked(&packed, a_slices, b_cols, len)
}

/// Pack the whole A panel into row-block-major order, ONCE, for reuse by every
/// worker of one `df.dot`.
///
/// `packed[block * k * MR + p * MR + i]` is `A[block * MR + i][p]`.
///
/// br-frankenpandas-mti15. Packing is what makes the register tile pay (the
/// unpacked tile measured 0.597x), but each worker packing the SAME A panel for
/// itself is 64x redundant copying on this host — `m * k * 8` bytes per worker,
/// 8 MB at `dim = 1000`, against a whole-op time of ~18 ms. The panel is
/// immutable and shared, so it can be built once before the scope and read by
/// every worker.
#[must_use]
pub fn pack_a_panel(a_slices: &[&[f64]], len: usize) -> Vec<f64> {
    let k = a_slices.len();
    let blocks = len / MR;
    let mut packed = vec![0.0_f64; blocks * k * MR];
    for block in 0..blocks {
        let row = block * MR;
        let base = block * k * MR;
        for (p, a) in a_slices.iter().enumerate() {
            packed[base + p * MR..base + p * MR + MR].copy_from_slice(&a[row..row + MR]);
        }
    }
    packed
}

/// [`materialize_float64_dot_block`] over a panel already packed by
/// [`pack_a_panel`].
///
/// `a_slices` is still required: the rows below the `MR` boundary are computed
/// from the original columns, in the same order the per-column kernel walks
/// them. Falls back to packing locally if `packed` is not the right length for
/// this shape, so a caller that passes a stale panel gets the right ANSWER
/// rather than a silent misread — the length check is cheap and the alternative
/// is reading another product's matrix.
#[inline(never)]
#[must_use]
pub fn materialize_float64_dot_block_prepacked(
    packed: &[f64],
    a_slices: &[&[f64]],
    b_cols: &[&[f64]],
    len: usize,
) -> Vec<Vec<f64>> {
    let n = b_cols.len();
    let k = a_slices.len();
    let mut out: Vec<Vec<f64>> = (0..n).map(|_| vec![0.0_f64; len]).collect();
    if n == 0 || k == 0 {
        return out;
    }
    let blocks = len / MR;
    let row_blocks = blocks * MR;
    let owned;
    let packed = if packed.len() == blocks * k * MR {
        packed
    } else {
        owned = pack_a_panel(a_slices, len);
        &owned
    };

    for block in 0..blocks {
        let row = block * MR;
        let base = block * k * MR;
        let panel = &packed[base..base + k * MR];
        let mut j0 = 0;
        while j0 < n {
            let nr = DOT_BLOCK_COLUMNS.min(n - j0);
            micro_kernel_packed(panel, b_cols, k, row, &mut out, j0, nr);
            j0 += DOT_BLOCK_COLUMNS;
        }
    }

    // Rows below the MR boundary, in the SAME accumulation order: outer over the
    // A columns, `p` ascending, exactly as the AXPY kernel walks them.
    for (column, b) in out.iter_mut().zip(b_cols.iter()) {
        for (a, &scale) in a_slices.iter().zip(b.iter()) {
            for (slot, &value) in column[row_blocks..len]
                .iter_mut()
                .zip(a[row_blocks..len].iter())
            {
                *slot += value * scale;
            }
        }
    }
    out
}

/// `out[i] = a[i] / b[i]` for every lane, returning whether ANY result is NaN.
///
/// br-frankenpandas-uza04. `div @1M` is a CERTIFIED 0.57x LOSS against live
/// pandas 2.2.3 (FP 640.60us vs pandas 348.41us, both 1 thread, A/A null FP
/// 0.98132 / pandas 0.99986). The cause is lane width, not the loop: the
/// baseline fp-columnar kernel disassembles to SSE2 `divpd`, two f64 per
/// instruction, while numpy runtime-dispatches a 4-lane AVX2 `vdivpd`. At ~2.6
/// cycles/element measured, FP is already at 2-lane `divpd` throughput — it is
/// not scalar and not stalled, it is simply half as wide.
///
/// Two OTHER hypotheses for this same row were measured and REJECTED first, so
/// this is not a third guess at the loop shape: the `collect` rewrite
/// (6b2147a7, 0.919x) and an integer-counter NaN witness (1.0153x against a
/// pre-registered 1.05x rule). Both left the instruction mix unchanged. Width is
/// what is left.
///
/// BIT-IDENTICAL to the baseline loop. IEEE-754 division is correctly rounded
/// per lane and lane-independent, so `vdivpd` and `divpd` produce the same bits
/// for the same inputs; widening changes how many divides retire per cycle, not
/// what any one of them returns. There is no reduction here to reassociate and
/// no `a*b+c` for `+fma` to contract, so the `+fma` half of this crate's profile
/// cannot alter the arithmetic either.
///
/// The NaN witness is folded into the same pass, exactly as the baseline kernel
/// does it, so the caller still gets `output_nan` without a second traversal
/// over the 8MB output — a second pass was measured and rejected on this very
/// row (docs/NEGATIVE_EVIDENCE.md 2026-08-17).
///
/// ⚠️ `#[inline(never)]` AND NON-GENERIC, for the reason documented on
/// [`materialize_float64_dot`]: an inlinable or generic entry point codegens in
/// the CALLER's baseline crate and the `+avx2` flag is silently lost, leaving a
/// green build with correct values and no speedup.
///
/// ⚠️ CALLER MUST GUARD with `is_x86_feature_detected!("avx2")`. This crate is
/// compiled for AVX2 unconditionally; entering it on a pre-AVX2 CPU is SIGILL.
///
/// # Panics
/// Panics if `a`, `b` and `out` do not all have the same length.
#[inline(never)]
pub fn div_f64_into(a: &[f64], b: &[f64], out: &mut [f64]) -> bool {
    assert_eq!(a.len(), b.len(), "div_f64_into: a/b length mismatch");
    assert_eq!(a.len(), out.len(), "div_f64_into: out length mismatch");

    // THE WITNESS FOLD, NOT THE DIVIDE, IS WHAT THIS KERNEL LOSES ON.
    //
    // `div @1M` is CERTIFIED SLOWER at 0.885x, and the divide is not the reason:
    // measured on one ELF, FP `add @1M` (same machinery, same 24MB of traffic,
    // same witness fold, no divider) is 301.8us against `div @1M` 398.6us, so the
    // divider is only 96.8us — 24%. pandas answers the whole divide in 334.84us,
    // barely more than FrankenPandas' shared overhead alone.
    //
    // Disassembling the previous scalar-source loop showed why the divider stays
    // EXPOSED here while it hides under memory latency for numpy. LLVM vectorised
    // it four lanes wide with four independent accumulators — no dependency chain,
    // that hypothesis was checked and refuted — but the `bool` fold made it narrow
    // every 256-bit compare back to 128 bits per lane:
    //
    //     vcmpunordpd %ymm1,%ymm5,%ymm9      <- the compare we actually want
    //     vextractf128 $0x1,%ymm9,%xmm10     <- pure narrowing overhead
    //     vpackssdw   %xmm10,%xmm9,%xmm9     <- pure narrowing overhead
    //     vpor        %xmm0,%xmm9,%xmm0
    //
    // Four instructions per four doubles for a witness, against one `vdivpd`.
    // Written as an explicit mask OR the fold is `vcmpXpd` + `vorpd`, both 256-bit,
    // so two of the four go away and the issue slots they occupied are free to
    // cover divide latency.
    //
    // `x != x` is NaN by IEEE, so `simd_ne(r, r)` is exactly `is_nan` lane-wise,
    // and lane-wise `av / bv` is the identical IEEE quotient the scalar loop
    // produced — the output is bit-identical and the witness is the same boolean.
    const LANES: usize = 4;
    let n = a.len();
    let chunk_end = n - n % LANES;
    let mut nan_acc = Mask::<i64, LANES>::splat(false);
    let mut index = 0usize;
    while index < chunk_end {
        let av = Simd::<f64, LANES>::from_slice(&a[index..index + LANES]);
        let bv = Simd::<f64, LANES>::from_slice(&b[index..index + LANES]);
        let r = av / bv;
        nan_acc |= r.simd_ne(r);
        r.copy_to_slice(&mut out[index..index + LANES]);
        index += LANES;
    }
    let mut output_nan = nan_acc.any();
    // Scalar remainder, identical to the original body.
    for offset in chunk_end..n {
        let r = a[offset] / b[offset];
        output_nan |= r.is_nan();
        out[offset] = r;
    }
    output_nan
}

/// The `+`, `-` and `*` siblings of [`div_f64_into`], generated from one body.
///
/// br-frankenpandas-uza04. `div` was given this crate's 4-lane width because
/// `div @1M` was a certified 0.885x loss; `add`, `sub` and `mul` share EVERY part
/// of that kernel — the same `&[f64]` in, the same contiguous store, the same
/// NaN witness fold — and were left on the baseline `+sse4.1` build, which is
/// 2-lane `addpd`/`subpd`/`mulpd` against numpy's 4-lane AVX2. Nothing about the
/// three ops made them a different case; `div` was simply the row that certified
/// in the window where the width work was done.
///
/// TWO SEPARATE GAINS, and the second is why these should beat the 1.2707x that a
/// blanket `+avx2` flag bought `add @1M` on 2026-08-17 (docs/NEGATIVE_EVIDENCE.md,
/// CrimsonPine, a row that measured the WIDTH only):
///
///   1. WIDTH. 4 lanes per instruction instead of 2.
///   2. THE WITNESS FOLD. That is the half the flag alone cannot reach. A `bool`
///      accumulator makes LLVM narrow every 256-bit compare back to 128 bits —
///      `vcmpunordpd` then `vextractf128` + `vpackssdw` + `vpor`, four
///      instructions per four doubles — while an explicit `Mask` fold is
///      `vcmpXpd` + `vorpd`, both 256-bit. The div comment above measures that
///      overhead directly, and it is proportionally LARGER here than it was
///      there: it is a fixed per-element cost sitting beside one `vaddpd` (~4
///      cycles) instead of beside one `vdivpd` (~13), so the fraction of the
///      loop it wastes is bigger for the cheap ops, not smaller.
///
/// BIT-IDENTICAL to the baseline loop, for the same reason `div` is: IEEE-754
/// `+`, `-` and `*` are each correctly rounded and lane-independent, so a 4-wide
/// `vaddpd` returns the same bits per lane as `addsd` on the same inputs. Width
/// changes how many retire per cycle, not what any one returns. There is no
/// reduction here to reassociate; and although this crate carries `+fma`, a lone
/// `a + b` / `a - b` / `a * b` has no `a * b + c` shape for the compiler to
/// contract, so the FMA half of the profile cannot alter the arithmetic either.
///
/// `x != x` is NaN by IEEE, so `simd_ne(r, r)` is exactly lane-wise `is_nan` and
/// the returned witness is the same boolean the scalar fold produces.
///
/// ⚠️ `#[inline(never)]` AND NON-GENERIC, for the reason documented on
/// [`materialize_float64_dot`]: an inlinable or generic entry point codegens in
/// the CALLER's baseline crate and the `+avx2` flag is silently lost, leaving a
/// green build with correct values and no speedup. That is why this is a macro
/// over three concrete `pub fn`s rather than one function generic over the op.
///
/// ⚠️ CALLER MUST GUARD with `is_x86_feature_detected!("avx2")`. This crate is
/// compiled for AVX2 unconditionally; entering it on a pre-AVX2 CPU is SIGILL.
macro_rules! elementwise_f64_kernel {
    ($name:ident, $simd_op:expr, $scalar_op:expr, $doc:literal) => {
        #[doc = $doc]
        ///
        /// Returns whether any output element was NaN. See
        /// [`elementwise_f64_kernel`] for the width/witness rationale and the
        /// bit-identity argument.
        ///
        /// # Panics
        /// Panics if `a`, `b` and `out` do not all have the same length.
        #[inline(never)]
        pub fn $name(a: &[f64], b: &[f64], out: &mut [f64]) -> bool {
            assert_eq!(
                a.len(),
                b.len(),
                concat!(stringify!($name), ": a/b length mismatch")
            );
            assert_eq!(
                a.len(),
                out.len(),
                concat!(stringify!($name), ": out length mismatch")
            );

            const LANES: usize = 4;
            let n = a.len();
            let chunk_end = n - n % LANES;
            let mut nan_acc = Mask::<i64, LANES>::splat(false);
            let mut index = 0usize;
            while index < chunk_end {
                let av = Simd::<f64, LANES>::from_slice(&a[index..index + LANES]);
                let bv = Simd::<f64, LANES>::from_slice(&b[index..index + LANES]);
                let r = $simd_op(av, bv);
                nan_acc |= r.simd_ne(r);
                r.copy_to_slice(&mut out[index..index + LANES]);
                index += LANES;
            }
            let mut output_nan = nan_acc.any();
            // Scalar remainder, identical to the baseline body.
            for offset in chunk_end..n {
                let r = $scalar_op(a[offset], b[offset]);
                output_nan |= r.is_nan();
                out[offset] = r;
            }
            output_nan
        }
    };
}

elementwise_f64_kernel!(
    add_f64_into,
    |x: Simd<f64, 4>, y: Simd<f64, 4>| x + y,
    |x: f64, y: f64| x + y,
    "Lane-wise `out[i] = a[i] + b[i]`, four doubles per instruction."
);
elementwise_f64_kernel!(
    sub_f64_into,
    |x: Simd<f64, 4>, y: Simd<f64, 4>| x - y,
    |x: f64, y: f64| x - y,
    "Lane-wise `out[i] = a[i] - b[i]`, four doubles per instruction."
);
elementwise_f64_kernel!(
    mul_f64_into,
    |x: Simd<f64, 4>, y: Simd<f64, 4>| x * y,
    |x: f64, y: f64| x * y,
    "Lane-wise `out[i] = a[i] * b[i]`, four doubles per instruction."
);

/// Lane-wise `out[i] = sqrt(a[i])`, four doubles per instruction, folding the
/// TWO witnesses `Column::sqrt`'s typed arm needs: `(domain_held, all_finite)`.
///
/// br-frankenpandas-uza04 / oxv4u. `sqrt @10M` measured **0.808x** against live
/// pandas 2.2.3 on 2026-08-30 (fp 19302.89us vs pandas 16101.55us, cv 7.46% /
/// 3.56%, `effect_ci_excludes_unity` and the A/A null BOTH true, thread count 1).
/// The mechanism is settled and is NOT "we lower to scalar `sqrtsd`" — that
/// attribution was checked and REFUTED (docs/NEGATIVE_EVIDENCE.md 2026-08-16
/// CrimsonPine): the baseline kernel already emits PACKED `sqrtpd`. It is simply
/// 2 lanes against numpy's 4-lane `vsqrtpd`, and `+sse4.1` has no 4-wide square
/// root to offer.
///
/// THE BLANKET ROUTE IS REJECTED AND THIS IS NOT IT. A global `x86-64-v3` policy
/// was measured and rejected (2026-07-31 CyanLynx: sqrt 0.361x, log regressed),
/// and `#[target_feature]` / local `allow(unsafe_code)` / portable-SIMD-at-
/// baseline were all compiled and are dead (E0133, E0453, and 4-wide vectors
/// lowering to 2xSSE2 respectively). Per-package rustflags on THIS crate is the
/// one mechanism that produced ymm registers, which is why the op comes here
/// rather than the flag going to fp-columnar.
///
/// WHY TWO WITNESSES AND NOT ONE. The caller
/// (`typed_float_domain_fused_unary_with_finiteness`, via `Column::sqrt`) fuses a
/// domain test and a finiteness fold into the same pass, and both must come back
/// or the caller pays a second traversal that this kernel exists to avoid:
///
///   * `domain_held` is `all(x >= 0.0)`. ⚠️ It is folded as `!(x >= 0.0)`, NOT as
///     `x < 0.0`. Those differ on NaN: `NaN < 0.0` is FALSE, so a `simd_lt` fold
///     would report a NaN input as IN domain and hand back `sqrt(NaN)` as a
///     PRESENT value, where the scalar predicate `|x| x >= 0.0` marks it missing.
///     That is a parity break, not a rounding difference.
///   * `all_finite` is `all(out[i].is_finite())`, folded as `!(|y| < inf)` so that
///     both NaN and +/-inf count as non-finite, matching `f64::is_finite`.
///
/// BIT-IDENTICAL to the baseline loop. IEEE-754 square root is correctly rounded
/// and lane-independent, so `vsqrtpd` returns the same bits per lane as `sqrtpd`
/// or `sqrtsd` on the same input; width changes throughput, not results. There is
/// no reduction to reassociate and no `a * b + c` for this crate's `+fma` to
/// contract.
///
/// ⚠️ `#[inline(never)]` and non-generic, for the reason on
/// [`materialize_float64_dot`]: an inlinable entry point codegens in the caller's
/// baseline crate and the `+avx2` flag is silently lost — a green build with
/// correct values and no speedup.
///
/// ⚠️ CALLER MUST GUARD with `is_x86_feature_detected!("avx2")`; this crate emits
/// AVX2 unconditionally and entering it on a pre-AVX2 CPU is SIGILL.
///
/// # Panics
/// Panics if `a` and `out` do not have the same length.
#[inline(never)]
pub fn sqrt_f64_into(a: &[f64], out: &mut [f64]) -> (bool, bool) {
    assert_eq!(a.len(), out.len(), "sqrt_f64_into: out length mismatch");

    const LANES: usize = 4;
    let n = a.len();
    let chunk_end = n - n % LANES;
    let zero = Simd::<f64, LANES>::splat(0.0);
    let inf = Simd::<f64, LANES>::splat(f64::INFINITY);
    let mut out_of_domain = Mask::<i64, LANES>::splat(false);
    let mut non_finite = Mask::<i64, LANES>::splat(false);
    let mut index = 0usize;
    while index < chunk_end {
        let av = Simd::<f64, LANES>::from_slice(&a[index..index + LANES]);
        // `!(x >= 0.0)` so NaN counts as OUT of domain; see the note above.
        out_of_domain |= !av.simd_ge(zero);
        let r = av.sqrt();
        non_finite |= !r.abs().simd_lt(inf);
        r.copy_to_slice(&mut out[index..index + LANES]);
        index += LANES;
    }
    let mut domain_held = !out_of_domain.any();
    let mut all_finite = !non_finite.any();
    // Scalar remainder, identical to the baseline body.
    for offset in chunk_end..n {
        let x = a[offset];
        domain_held &= x >= 0.0;
        let r = x.sqrt();
        all_finite &= r.is_finite();
        out[offset] = r;
    }
    (domain_held, all_finite)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SIMD witness fold must agree with the scalar `is_nan` fold on every
    /// element AND on the returned boolean — including the cases a cheaper fold
    /// would get wrong. `inf` outputs are PRESENT values under pandas semantics,
    /// so a witness built from e.g. `r - r` (NaN for both NaN and inf) would
    /// wrongly mark valid data missing; this pins that.
    #[test]
    fn div_simd_witness_matches_the_scalar_fold_including_inf_and_remainder() {
        fn scalar(a: &[f64], b: &[f64]) -> (Vec<f64>, bool) {
            let mut out = vec![0.0; a.len()];
            let mut nan = false;
            for i in 0..a.len() {
                let r = a[i] / b[i];
                nan |= r.is_nan();
                out[i] = r;
            }
            (out, nan)
        }

        // Lengths straddling the 4-lane chunk so the scalar remainder runs too.
        for len in [0usize, 1, 3, 4, 5, 7, 8, 9, 16, 17, 1000, 1001] {
            let mut a = Vec::with_capacity(len);
            let mut b = Vec::with_capacity(len);
            let mut state: u64 = 0x2545_F491_4F6C_DD1D;
            for i in 0..len {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let unit = ((state >> 11) as f64) / ((1u64 << 53) as f64);
                // Seed the interesting shapes: x/0 -> +-inf (PRESENT), 0/0 -> NaN,
                // inf/inf -> NaN, plus ordinary values.
                match i % 11 {
                    0 => {
                        a.push(1.0);
                        b.push(0.0);
                    }
                    1 => {
                        a.push(-1.0);
                        b.push(0.0);
                    }
                    2 => {
                        a.push(f64::INFINITY);
                        b.push(f64::INFINITY);
                    }
                    3 => {
                        a.push(0.0);
                        b.push(0.0);
                    }
                    _ => {
                        a.push(unit * 1e6 - 5e5);
                        b.push(unit * 7.0 + 0.5);
                    }
                }
            }
            let (want, want_nan) = scalar(&a, &b);
            let mut got = vec![0.0; len];
            let got_nan = div_f64_into(&a, &b, &mut got);
            assert_eq!(got_nan, want_nan, "witness disagrees at len {len}");
            for i in 0..len {
                assert_eq!(
                    got[i].to_bits(),
                    want[i].to_bits(),
                    "value disagrees at len {len} index {i}"
                );
            }
        }

        // An all-inf, no-NaN case must report NO nan: inf is present, not missing.
        let a = vec![1.0f64; 8];
        let b = vec![0.0f64; 8];
        let mut out = vec![0.0; 8];
        assert!(
            !div_f64_into(&a, &b, &mut out),
            "x/0 is +inf, a PRESENT value; the witness must not claim NaN"
        );
        assert!(out.iter().all(|v| *v == f64::INFINITY));
    }

    /// br-frankenpandas-uza04. Same obligation the `div` test above discharges,
    /// for the three siblings: bit-identical values and an identical witness
    /// against the scalar fold, at lengths that straddle the 4-lane chunk so the
    /// remainder loop is exercised too.
    ///
    /// THE FIXTURE IS BUILT FOR THESE OPS, not copied from `div`. The shapes that
    /// can mint a NaN differ per op and a fixture that never produces one would
    /// pass while testing nothing about the witness:
    ///   * `inf + -inf`, `inf - inf`     -> NaN
    ///   * `0 * inf`                     -> NaN
    ///   * `inf + inf`, `inf * inf`      -> inf, a PRESENT value the witness must
    ///                                      NOT flag (the same trap the div test
    ///                                      pins with `x/0`)
    ///   * a NaN operand                 -> NaN, propagated by all three
    #[test]
    fn add_sub_mul_simd_witnesses_match_the_scalar_fold_including_inf_and_remainder() {
        fn scalar(a: &[f64], b: &[f64], op: fn(f64, f64) -> f64) -> (Vec<f64>, bool) {
            let mut out = vec![0.0; a.len()];
            let mut nan = false;
            for i in 0..a.len() {
                let r = op(a[i], b[i]);
                nan |= r.is_nan();
                out[i] = r;
            }
            (out, nan)
        }

        type Kernel = fn(&[f64], &[f64], &mut [f64]) -> bool;
        let cases: [(&str, Kernel, fn(f64, f64) -> f64); 3] = [
            ("add", add_f64_into, |x, y| x + y),
            ("sub", sub_f64_into, |x, y| x - y),
            ("mul", mul_f64_into, |x, y| x * y),
        ];

        for (name, kernel, scalar_op) in cases {
            for len in [0usize, 1, 3, 4, 5, 7, 8, 9, 16, 17, 1000, 1001] {
                let mut a = Vec::with_capacity(len);
                let mut b = Vec::with_capacity(len);
                let mut state: u64 = 0x2545_F491_4F6C_DD1D;
                for i in 0..len {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    let unit = ((state >> 11) as f64) / ((1u64 << 53) as f64);
                    match i % 11 {
                        // inf + -inf and inf - inf are NaN; inf * -inf is -inf.
                        0 => {
                            a.push(f64::INFINITY);
                            b.push(f64::NEG_INFINITY);
                        }
                        // inf - inf is NaN; inf + inf and inf * inf are +inf,
                        // PRESENT values the witness must not flag.
                        1 => {
                            a.push(f64::INFINITY);
                            b.push(f64::INFINITY);
                        }
                        // 0 * inf is NaN; 0 +/- inf is +/-inf.
                        2 => {
                            a.push(0.0);
                            b.push(f64::INFINITY);
                        }
                        // A NaN operand: propagated by all three ops.
                        3 => {
                            a.push(f64::NAN);
                            b.push(unit);
                        }
                        // Signed zero, which `sub` distinguishes: 0.0 - 0.0 is
                        // +0.0 but 0.0 + -0.0 is +0.0 and -0.0 + -0.0 is -0.0.
                        // Compared by BITS below, so a sign slip is caught.
                        4 => {
                            a.push(-0.0);
                            b.push(0.0);
                        }
                        // Overflow to inf, and denormal underflow to zero.
                        5 => {
                            a.push(f64::MAX);
                            b.push(f64::MAX);
                        }
                        6 => {
                            a.push(f64::MIN_POSITIVE);
                            b.push(f64::MIN_POSITIVE);
                        }
                        _ => {
                            a.push(unit * 1e6 - 5e5);
                            b.push(unit * 7.0 + 0.5);
                        }
                    }
                }
                let (want, want_nan) = scalar(&a, &b, scalar_op);
                let mut got = vec![0.0; len];
                let got_nan = kernel(&a, &b, &mut got);
                assert_eq!(got_nan, want_nan, "{name}: witness disagrees at len {len}");
                for i in 0..len {
                    assert_eq!(
                        got[i].to_bits(),
                        want[i].to_bits(),
                        "{name}: value disagrees at len {len} index {i}"
                    );
                }
            }
        }

        // NON-VACUITY: the fixture above must actually MINT a NaN for each op,
        // or the witness assertions passed against `false == false` and proved
        // nothing. Checked positively rather than assumed.
        let a = [f64::INFINITY, 0.0];
        let b = [f64::NEG_INFINITY, f64::INFINITY];
        let mut out = [0.0; 2];
        assert!(add_f64_into(&a, &b, &mut out), "inf + -inf must be NaN");
        assert!(
            sub_f64_into(&[f64::INFINITY], &[f64::INFINITY], &mut out[..1]),
            "inf - inf must be NaN"
        );
        assert!(
            mul_f64_into(&[0.0], &[f64::INFINITY], &mut out[..1]),
            "0 * inf must be NaN"
        );

        // And the mirror of the div test's trap: an all-inf, no-NaN result must
        // report NO nan, because inf is a PRESENT value under pandas semantics.
        let a = vec![f64::INFINITY; 8];
        let b = vec![f64::INFINITY; 8];
        let mut out = vec![0.0; 8];
        assert!(
            !add_f64_into(&a, &b, &mut out),
            "inf + inf is +inf, a PRESENT value; the witness must not claim NaN"
        );
        assert!(out.iter().all(|v| *v == f64::INFINITY));
        assert!(
            !mul_f64_into(&a, &b, &mut out),
            "inf * inf is +inf, a PRESENT value; the witness must not claim NaN"
        );
        assert!(out.iter().all(|v| *v == f64::INFINITY));
    }

    /// br-frankenpandas-uza04. `sqrt_f64_into` must agree with the SCALAR fold on
    /// values AND on both witnesses, at lengths straddling the 4-lane chunk.
    ///
    /// THE NaN CASE IS THE WHOLE POINT of this test. `domain_held` is
    /// `all(x >= 0.0)`, and a natural-looking `simd_lt(zero)` fold gets NaN
    /// BACKWARDS — `NaN < 0.0` is false, so NaN would read as in-domain and
    /// `sqrt(NaN)` would be handed back as a PRESENT value where the caller's
    /// predicate marks it missing. The fixture therefore contains NaN, and a
    /// non-vacuity block below proves each witness actually goes false somewhere.
    #[test]
    fn sqrt_kernel_matches_the_scalar_fold_on_values_and_both_witnesses() {
        fn scalar(a: &[f64]) -> (Vec<f64>, bool, bool) {
            let mut out = vec![0.0; a.len()];
            let (mut domain, mut finite) = (true, true);
            for i in 0..a.len() {
                domain &= a[i] >= 0.0;
                let r = a[i].sqrt();
                finite &= r.is_finite();
                out[i] = r;
            }
            (out, domain, finite)
        }

        // Case sets chosen so the two witnesses vary INDEPENDENTLY: all-positive
        // (both true), with +inf (domain true, finite false), with a negative
        // (domain false), with NaN (domain false via the `!(x >= 0)` fold).
        let shapes: [(&str, fn(usize) -> f64); 4] = [
            ("positive", |i| (i as f64) * 1.5 + 0.25),
            ("with_inf", |i| {
                if i % 7 == 3 {
                    f64::INFINITY
                } else {
                    (i as f64) + 1.0
                }
            }),
            ("with_negative", |i| {
                if i % 5 == 2 {
                    -(i as f64) - 1.0
                } else {
                    (i as f64) + 1.0
                }
            }),
            ("with_nan", |i| {
                if i % 6 == 4 {
                    f64::NAN
                } else {
                    (i as f64) + 1.0
                }
            }),
        ];

        for (name, make) in shapes {
            for len in [0usize, 1, 3, 4, 5, 7, 8, 9, 16, 17, 1000, 1001] {
                let a: Vec<f64> = (0..len).map(make).collect();
                let (want, want_domain, want_finite) = scalar(&a);
                let mut got = vec![0.0; len];
                let (got_domain, got_finite) = sqrt_f64_into(&a, &mut got);
                assert_eq!(
                    got_domain, want_domain,
                    "{name}: domain witness at len {len}"
                );
                assert_eq!(
                    got_finite, want_finite,
                    "{name}: finite witness at len {len}"
                );
                for i in 0..len {
                    assert_eq!(
                        got[i].to_bits(),
                        want[i].to_bits(),
                        "{name}: value at len {len} index {i}"
                    );
                }
            }
        }

        // NON-VACUITY: each witness must actually be observed FALSE, or the
        // equality assertions above passed against `true == true` throughout.
        let mut out = [0.0; 8];
        assert!(
            !sqrt_f64_into(&[-1.0; 8], &mut out).0,
            "negative must clear domain"
        );
        assert!(
            !sqrt_f64_into(&[f64::NAN; 8], &mut out).0,
            "NaN must clear domain: `!(x >= 0.0)`, NOT `x < 0.0`"
        );
        assert!(
            !sqrt_f64_into(&[f64::INFINITY; 8], &mut out).1,
            "sqrt(inf) is inf, which is NOT finite"
        );
        let both = sqrt_f64_into(&[4.0; 8], &mut out);
        assert_eq!(both, (true, true), "a clean positive column must set both");
        assert!(out.iter().all(|v| *v == 2.0));
    }

    #[test]
    fn matches_a_scalar_reference_sum() {
        let a0 = [1.0_f64, 2.0, 3.0];
        let a1 = [10.0_f64, 20.0, 30.0];
        let slices: [&[f64]; 2] = [&a0, &a1];
        let b = [2.0_f64, 0.5];
        let got = materialize_float64_dot(&slices, &b, 3);
        // out[r] = a0[r]*2.0 + a1[r]*0.5
        assert_eq!(
            got,
            vec![
                1.0 * 2.0 + 10.0 * 0.5,
                2.0 * 2.0 + 20.0 * 0.5,
                3.0 * 2.0 + 30.0 * 0.5
            ]
        );
    }

    #[test]
    fn empty_inputs_yield_zeros() {
        let got = materialize_float64_dot(&[], &[], 4);
        assert_eq!(got, vec![0.0; 4]);
    }

    /// Deterministic xorshift, so the identity below is exercised on values that
    /// actually round — a table of small exact integers cannot distinguish two
    /// summation orders, which is precisely what this test exists to pin.
    fn pseudo_random(count: usize, seed: u64) -> Vec<f64> {
        let mut state = seed;
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ((state >> 11) as f64) / ((1_u64 << 53) as f64) - 0.5
            })
            .collect()
    }

    /// THE LOAD-BEARING TEST for `materialize_float64_dot_block`, and it is a
    /// BIT test on purpose: the blocked kernel's whole claim is that it changes
    /// the memory schedule and NOT the arithmetic. A tolerance-based comparison
    /// would pass a kernel that reassociated the `k` sum or used `mul_add`, which
    /// is exactly the change this repo has reverted before (49a8fa5ed withdrew a
    /// blocked-summation lever for breaking a bit-identity gate).
    ///
    /// The shapes are chosen to hit every boundary the kernel has: `len` on and
    /// off the `MR = 8` row boundary (the scalar tail runs or does not), and `n`
    /// on, above and below `DOT_BLOCK_COLUMNS = 6` (a full column block, a
    /// partial one, and both together).
    #[test]
    fn blocked_kernel_is_bit_identical_to_the_per_column_kernel() {
        for &(len, k, n) in &[
            (1_usize, 1_usize, 1_usize),
            (7, 3, 5),
            (8, 4, 6),
            (9, 5, 7),
            (16, 16, 12),
            (37, 11, 13),
            (64, 33, 6),
        ] {
            let a_store: Vec<Vec<f64>> = (0..k)
                .map(|j| pseudo_random(len, 0x2545_F491_4F6C_DD1D ^ (j as u64 + 1)))
                .collect();
            let b_store: Vec<Vec<f64>> = (0..n)
                .map(|j| pseudo_random(k, 0x9E37_79B9_7F4A_7C15 ^ (j as u64 + 1)))
                .collect();
            let a_slices: Vec<&[f64]> = a_store.iter().map(Vec::as_slice).collect();
            let b_slices: Vec<&[f64]> = b_store.iter().map(Vec::as_slice).collect();

            let blocked = materialize_float64_dot_block(&a_slices, &b_slices, len);
            assert_eq!(blocked.len(), n);
            for (column, b) in blocked.iter().zip(b_slices.iter()) {
                let reference = materialize_float64_dot(&a_slices, b, len);
                for (row, (got, want)) in column.iter().zip(reference.iter()).enumerate() {
                    assert_eq!(
                        got.to_bits(),
                        want.to_bits(),
                        "len={len} k={k} n={n} row={row}: blocked {got:e} vs per-column {want:e}"
                    );
                }
            }
        }
    }

    /// NON-VACUITY for the test above: a kernel that reassociated the `k` sum
    /// would still agree to ~1e-15, so prove the inputs can actually TELL two
    /// orders apart. Summing the same products forwards and backwards must
    /// differ in the low bits — if it does not, the fixture is too clean and the
    /// identity test above is asserting nothing.
    #[test]
    fn the_fixture_can_distinguish_two_summation_orders() {
        let a = pseudo_random(512, 0x1234_5678_9ABC_DEF0);
        let b = pseudo_random(512, 0x0FED_CBA9_8765_4321);
        let forward = a.iter().zip(b.iter()).fold(0.0_f64, |s, (x, y)| s + x * y);
        let backward = a
            .iter()
            .zip(b.iter())
            .rev()
            .fold(0.0_f64, |s, (x, y)| s + x * y);
        assert_ne!(
            forward.to_bits(),
            backward.to_bits(),
            "the pseudo-random fixture rounds identically in both directions, so a \
             bit-identity assertion over it would be vacuous"
        );
    }

    /// The blocked kernel must survive the degenerate shapes the per-column one
    /// already handles, rather than panicking on an empty panel.
    #[test]
    fn blocked_kernel_handles_empty_panels() {
        assert!(materialize_float64_dot_block(&[], &[], 4).is_empty());
        let b: [f64; 0] = [];
        let b_slices: [&[f64]; 1] = [&b];
        assert_eq!(
            materialize_float64_dot_block(&[], &b_slices, 3),
            vec![vec![0.0_f64; 3]]
        );
    }
}

#[cfg(test)]
mod div_f64_into_uza04 {
    //! The AVX2 divide must equal the baseline scalar loop BIT FOR BIT, including
    //! on the special values where a "reasonable" divide differs from IEEE-754.
    //!
    //! br-frankenpandas-uza04. This kernel exists only to widen `divpd` (2 lanes)
    //! to `vdivpd` (4 lanes) on the certified `div @1M` 0.57x loss. Widening must
    //! not change a single result, so these tests compare against a reference
    //! computed the ordinary way rather than against hand-written expectations —
    //! a shared misconception cannot then cancel out.
    //!
    //! THE FAILURE MODE THESE EXIST FOR: a vector divide processes lanes in
    //! blocks, so a bug in the remainder handling corrupts only the tail — the
    //! last 1..3 elements of a non-multiple-of-4 length. Every length below is
    //! therefore checked, not just round ones.

    /// The baseline fp-columnar loop, reproduced exactly.
    fn reference(a: &[f64], b: &[f64]) -> (Vec<f64>, bool) {
        let mut out = vec![0.0_f64; a.len()];
        let mut nan = false;
        for ((x, y), d) in a.iter().zip(b.iter()).zip(out.iter_mut()) {
            let r = x / y;
            nan |= r.is_nan();
            *d = r;
        }
        (out, nan)
    }

    fn assert_bit_identical(a: &[f64], b: &[f64]) {
        let (want, want_nan) = reference(a, b);
        let mut got = vec![0.0_f64; a.len()];
        let got_nan = super::div_f64_into(a, b, &mut got);
        assert_eq!(got_nan, want_nan, "NaN witness disagreed (len {})", a.len());
        for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(
                g.to_bits(),
                w.to_bits(),
                "lane {i} of {} differs: {g:?} vs {w:?}",
                a.len()
            );
        }
    }

    /// Every length from 0 to 33 — covers all four remainder classes of a 4-wide
    /// body plus the empty and sub-vector cases.
    #[test]
    fn every_tail_length_is_bit_identical() {
        for n in 0..34_usize {
            let a: Vec<f64> = (0..n).map(|i| (i as f64) * 1.7 - 11.0).collect();
            let b: Vec<f64> = (0..n).map(|i| (i as f64) * 0.3 + 1.1).collect();
            assert_bit_identical(&a, &b);
        }
    }

    /// IEEE-754 division corner cases: x/0 is ±inf, 0/0 and inf/inf are NaN,
    /// signed zeros carry sign, and subnormals must not be flushed.
    #[test]
    fn ieee_special_values_are_bit_identical() {
        let a = [
            1.0,
            -1.0,
            0.0,
            -0.0,
            0.0,
            f64::INFINITY,
            f64::INFINITY,
            f64::NAN,
            1.0,
            f64::MIN_POSITIVE,
            5e-324,
            -3.5,
        ];
        let b = [
            0.0,
            0.0,
            0.0,
            0.0,
            -0.0,
            f64::INFINITY,
            2.0,
            1.0,
            f64::NAN,
            2.0,
            2.0,
            -0.0,
        ];
        assert_bit_identical(&a, &b);
    }

    /// The witness is FALSE when no result is NaN and TRUE when one is, including
    /// when the only NaN is produced in the tail rather than the vector body.
    #[test]
    fn nan_witness_tracks_the_results_including_in_the_tail() {
        let clean_a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let clean_b = [2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 2.0];
        let mut out = vec![0.0_f64; 7];
        assert!(!super::div_f64_into(&clean_a, &clean_b, &mut out));

        // 0.0/0.0 = NaN placed at index 6, the tail of a 4-wide body.
        let tail_a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0];
        let tail_b = [2.0, 2.0, 2.0, 2.0, 2.0, 2.0, 0.0];
        assert!(super::div_f64_into(&tail_a, &tail_b, &mut out));
        assert_bit_identical(&tail_a, &tail_b);
    }

    /// A length past any plausible unroll factor, with a NaN only near the end.
    #[test]
    fn long_run_with_late_nan_is_bit_identical() {
        let n = 1021_usize; // prime, so no unroll factor divides it
        let mut a: Vec<f64> = (0..n).map(|i| (i as f64) * 0.5 + 1.0).collect();
        let mut b: Vec<f64> = (0..n).map(|i| ((i % 7) as f64) - 3.0).collect();
        a[n - 2] = 0.0;
        b[n - 2] = 0.0;
        assert_bit_identical(&a, &b);
    }
}
