use serde::{Deserialize, Serialize};

use crate::asupersync::error::AsupersyncError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityProof {
    pub algorithm: String,
    pub expected_digest: String,
    pub observed_digest: String,
    pub verified: bool,
}

pub trait IntegrityVerifier {
    fn verify(
        &self,
        artifact_id: &str,
        bytes: &[u8],
        expected_digest: &str,
    ) -> Result<IntegrityProof, AsupersyncError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Fnv1aVerifier;

impl IntegrityVerifier for Fnv1aVerifier {
    fn verify(
        &self,
        artifact_id: &str,
        bytes: &[u8],
        expected_digest: &str,
    ) -> Result<IntegrityProof, AsupersyncError> {
        let observed_digest = fnv1a_hex(bytes);
        if observed_digest != expected_digest {
            return Err(AsupersyncError::IntegrityMismatch {
                artifact_id: artifact_id.to_string(),
                expected: expected_digest.to_string(),
                observed: observed_digest,
            });
        }

        Ok(IntegrityProof {
            algorithm: "fnv1a64".to_string(),
            expected_digest: expected_digest.to_string(),
            observed_digest,
            verified: true,
        })
    }
}

fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut digest = String::with_capacity(16);
    for shift in (0..16).rev().map(|nibble| nibble * 4) {
        let nibble = ((hash >> shift) & 0x0f) as usize;
        digest.push(char::from(HEX[nibble]));
    }
    digest
}

#[cfg(test)]
mod tests {
    use std::{
        hint::black_box,
        time::{Duration, Instant},
    };

    use super::{Fnv1aVerifier, IntegrityVerifier, fnv1a_hex};

    fn former_fnv1a_hex(bytes: &[u8]) -> String {
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }

    #[test]
    fn fnv1a_hex_matches_former_formatter_on_digest_boundaries() {
        for bytes in [
            b"".as_slice(),
            b"a".as_slice(),
            b"hello world".as_slice(),
            &[0_u8, 1, 15, 16, 127, 128, 255],
        ] {
            assert_eq!(fnv1a_hex(bytes), former_fnv1a_hex(bytes));
        }

        assert_eq!(fnv1a_hex(b""), "cbf29ce484222325");
        assert_eq!(fnv1a_hex(b"hello"), "a430d84680aabd0b");
    }

    #[test]
    fn verifier_preserves_exact_digest_and_mismatch_payload() {
        let verifier = Fnv1aVerifier;
        let bytes = b"integrity payload";
        let expected = fnv1a_hex(bytes);
        let proof = verifier.verify("artifact", bytes, &expected).unwrap();
        assert_eq!(proof.expected_digest, expected);
        assert_eq!(proof.observed_digest, expected);

        let mismatch = verifier.verify("artifact", bytes, "0000000000000000");
        assert!(matches!(
            mismatch,
            Err(crate::asupersync::error::AsupersyncError::IntegrityMismatch {
                artifact_id,
                expected,
                observed,
            }) if artifact_id == "artifact"
                && expected == "0000000000000000"
                && observed == fnv1a_hex(bytes)
        ));
    }

    #[test]
    #[ignore = "foreground release attribution harness"]
    fn fnv1a_hex_fixed_width_tail_ab_1elys() {
        const BATCH: usize = 16_384;
        const BLOCKS: usize = 21;
        const BOOTSTRAPS: usize = 2_000;

        fn elapsed(bytes: &[u8], render: fn(&[u8]) -> String) -> Duration {
            let started = Instant::now();
            let mut last = 0_u8;
            for _ in 0..BATCH {
                let digest = black_box(render(black_box(bytes)));
                last ^= digest.as_bytes().last().copied().unwrap_or_default();
                black_box(&digest);
            }
            black_box(last);
            started.elapsed()
        }

        fn median(values: &mut [f64]) -> f64 {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        }

        fn bootstrap_median_ci(samples: &[f64]) -> (f64, f64) {
            let mut state = 0x9e37_79b9_7f4a_7c15_u64;
            let mut medians = Vec::with_capacity(BOOTSTRAPS);
            let mut resample = vec![0.0; samples.len()];
            for _ in 0..BOOTSTRAPS {
                for value in &mut resample {
                    state ^= state << 7;
                    state ^= state >> 9;
                    *value = samples[(state as usize) % samples.len()];
                }
                medians.push(median(&mut resample));
            }
            medians.sort_by(f64::total_cmp);
            (
                medians[BOOTSTRAPS / 40],
                medians[BOOTSTRAPS - BOOTSTRAPS / 40 - 1],
            )
        }

        let bytes = b"integrity verifier fixed-width digest tail";
        assert_eq!(former_fnv1a_hex(bytes), fnv1a_hex(bytes));

        let mut aa_ratios = Vec::with_capacity(BLOCKS);
        let mut ab_ratios = Vec::with_capacity(BLOCKS);
        for block in 0..BLOCKS {
            let (former, candidate) = if block.is_multiple_of(2) {
                (elapsed(bytes, former_fnv1a_hex), elapsed(bytes, fnv1a_hex))
            } else {
                let candidate = elapsed(bytes, fnv1a_hex);
                let former = elapsed(bytes, former_fnv1a_hex);
                (former, candidate)
            };
            let candidate_per_call = candidate.as_secs_f64() / BATCH as f64;
            let former_per_call = former.as_secs_f64() / BATCH as f64;
            ab_ratios.push(former_per_call / candidate_per_call);

            let first = elapsed(bytes, former_fnv1a_hex).as_secs_f64() / BATCH as f64;
            let second = elapsed(bytes, former_fnv1a_hex).as_secs_f64() / BATCH as f64;
            aa_ratios.push(first / second);
        }

        let aa_median = median(&mut aa_ratios);
        let ab_median = median(&mut ab_ratios);
        let (aa_low, aa_high) = bootstrap_median_ci(&aa_ratios);
        let (ab_low, ab_high) = bootstrap_median_ci(&ab_ratios);
        let executable = std::fs::read("/proc/self/exe").expect("read executing test ELF");
        let elf_sha256 = crate::sha256_hex(&executable);
        eprintln!(
            "FNV1A_HEX_1ELYS elf_sha256={elf_sha256} batch={BATCH} blocks={BLOCKS} aa_median={aa_median:.4} aa_median_ci95=[{aa_low:.4},{aa_high:.4}] candidate_speedup_median={ab_median:.4} candidate_speedup_median_ci95=[{ab_low:.4},{ab_high:.4}]"
        );
        assert!(
            (0.95..=1.05).contains(&aa_median) && aa_low <= 1.0 && aa_high >= 1.0,
            "A/A null gate failed: median={aa_median:.4}, CI=[{aa_low:.4},{aa_high:.4}]"
        );
    }
}
