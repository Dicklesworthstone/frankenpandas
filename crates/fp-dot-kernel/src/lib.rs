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

#[cfg(test)]
mod tests {
    use super::*;

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
