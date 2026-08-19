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
        assert_eq!(got, vec![1.0 * 2.0 + 10.0 * 0.5, 2.0 * 2.0 + 20.0 * 0.5, 3.0 * 2.0 + 30.0 * 0.5]);
    }

    #[test]
    fn empty_inputs_yield_zeros() {
        let got = materialize_float64_dot(&[], &[], 4);
        assert_eq!(got, vec![0.0; 4]);
    }
}
