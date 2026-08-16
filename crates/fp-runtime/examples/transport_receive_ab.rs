//! br-frankenpandas-3nzz3 — A/B for `InMemoryTransport::receive`'s critical section.
//!
//! The structural change shipped long ago (07dcf4f96, repaired at 38df4882d):
//! `receive` stores `Arc<EncodedArtifact>`, copies the `Arc` under the mutex, and
//! performs the DEEP clone after unlocking. The bead stayed open because that is
//! a SPEED claim with no number attached, and its own acceptance text demands a
//! strict-remote release A/B with a null control before anything may be banked.
//!
//! This harness supplies the number. It is deliberately self-contained in
//! `fp-runtime`: `fp-bench` is a vs-pandas harness and it depends on `fp-frame`,
//! which is the wrong shape for an internal concurrency comparison and, at the
//! time of writing, not always buildable.
//!
//! WHAT IS COMPARED
//!
//!   SHIPPED  `fp_runtime::asupersync::transport::InMemoryTransport`
//!            Arc clone under the lock, deep clone outside it.
//!   LEGACY   `LegacyInMemoryTransport` below — the pre-3nzz3 shape, reproduced
//!            here as a CONTROL. It deep-clones the artifact WHILE HOLDING the
//!            mutex, which is exactly the critical section the lever shortened.
//!
//! The control lives in this harness and not in production code, so the shipped
//! path is measured exactly as it ships.
//!
//! WHY IT IS MEASURED CONCURRENTLY
//!
//! Both shapes do the same total work per call; only the mutex HOLD TIME
//! differs. A single-threaded loop therefore cannot see the lever at all — it
//! would report ~1.0x and look like a refutation. Threads all pull the SAME
//! artifact id, which is maximum contention and the condition the lever exists
//! for. A sequential (1-thread) row is printed alongside as the negative
//! control: the lever SHOULD be ~1.0x there, and a large sequential effect would
//! mean this harness is measuring something other than lock hold time.
//!
//! MEASUREMENT DISCIPLINE (campaign law)
//!
//!   * Both arms run in the SAME invocation on the SAME worker. A ratio built
//!     from two invocations is not evidence — the same cell has read 1.2693x and
//!     0.0093x on two workers with both A/A nulls passing.
//!   * Arms are INTERLEAVED in a balanced square (ABBA / BAAB by round) so drift
//!     and position effects cancel instead of loading onto one arm.
//!   * An A/A null runs the shipped arm against itself through the identical
//!     path. A null far from 1.0 invalidates the run.
//!   * The row names the WORKER and the HARNESS (this file's own SHA-256, since
//!     two sanctioned harnesses have disagreed ~2x on one worker with both nulls
//!     green), plus thread count, governor and ISA.
//!
//! Run:
//!   RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- \
//!     cargo run --release -p fp-runtime --example transport_receive_ab

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use fp_runtime::asupersync::{
    codec::EncodedArtifact,
    config::{AsupersyncConfig, CapabilitySet, CxCapability},
    error::AsupersyncError,
    transport::{InMemoryTransport, TransportLayer},
    validate_capability_gate,
};

/// The pre-br-frankenpandas-3nzz3 shape, reproduced as a control.
///
/// The ONLY intended difference from the shipped transport is where the deep
/// clone happens: here it is inside the `guard` scope, so the `Vec<u8>` copy is
/// serialized across every concurrent caller.
#[derive(Debug, Clone, Default)]
struct LegacyInMemoryTransport {
    storage: Arc<Mutex<BTreeMap<String, EncodedArtifact>>>,
}

impl LegacyInMemoryTransport {
    fn required_capabilities() -> CapabilitySet {
        CapabilitySet::for_capability(CxCapability::Io)
            .union(CapabilitySet::for_capability(CxCapability::Remote))
    }

    fn store(&self, artifact: EncodedArtifact, config: &AsupersyncConfig) {
        validate_capability_gate(config, Self::required_capabilities()).expect("capability gate");
        self.storage
            .lock()
            .expect("legacy transport lock")
            .insert(artifact.artifact_id.clone(), artifact);
    }

    fn receive(
        &self,
        artifact_id: &str,
        config: &AsupersyncConfig,
    ) -> Result<EncodedArtifact, AsupersyncError> {
        validate_capability_gate(config, Self::required_capabilities())?;
        let guard = self.storage.lock().map_err(|_| {
            AsupersyncError::Transport("in-memory transport lock poisoned".to_string())
        })?;
        // THE CONTROL'S DEFINING PROPERTY: the deep clone is inside the guard.
        guard
            .get(artifact_id)
            .cloned()
            .ok_or_else(|| AsupersyncError::ArtifactNotFound(artifact_id.to_string()))
    }
}

const ARTIFACT_ID: &str = "transport-artifact-3nzz3";

fn artifact(payload_bytes: usize) -> EncodedArtifact {
    // A deterministic, incompressible-ish payload. Size is the whole point: the
    // lever moves a Vec<u8> copy out of the critical section, so the effect
    // scales with how long that copy takes.
    let encoded_bytes: Vec<u8> = (0..payload_bytes).map(|i| (i % 251) as u8).collect();
    EncodedArtifact {
        artifact_id: ARTIFACT_ID.to_string(),
        source_len: payload_bytes,
        encoded_bytes,
        repair_symbols: 1,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arm {
    Shipped,
    Legacy,
}

fn run_arm(
    arm: Arm,
    shipped: &InMemoryTransport,
    legacy: &LegacyInMemoryTransport,
    config: &AsupersyncConfig,
    threads: usize,
    receives_per_thread: usize,
) -> Duration {
    let start = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut checksum: u64 = 0;
                for _ in 0..receives_per_thread {
                    let got = match arm {
                        Arm::Shipped => shipped.receive(ARTIFACT_ID, config),
                        Arm::Legacy => legacy.receive(ARTIFACT_ID, config),
                    }
                    .expect("receive must succeed");
                    // Consume the result so neither arm can be optimized into
                    // nothing, and so both pay the same read cost.
                    checksum = checksum
                        .wrapping_add(got.encoded_bytes[0] as u64)
                        .wrapping_add(got.encoded_bytes[got.encoded_bytes.len() - 1] as u64)
                        .wrapping_add(got.source_len as u64);
                }
                std::hint::black_box(checksum);
            });
        }
    });
    start.elapsed()
}

/// Median of a set of durations, in nanoseconds.
fn median_ns(mut samples: Vec<Duration>) -> f64 {
    samples.sort();
    let n = samples.len();
    if n == 0 {
        return f64::NAN;
    }
    if n % 2 == 1 {
        samples[n / 2].as_nanos() as f64
    } else {
        (samples[n / 2 - 1].as_nanos() as f64 + samples[n / 2].as_nanos() as f64) / 2.0
    }
}

fn read_trimmed(path: &str) -> String {
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "unknown".to_owned())
}

fn harness_sha256() -> String {
    use sha2::{Digest, Sha256};
    // Hash THIS SOURCE FILE, not the binary: the binary's hash moves with every
    // dependency rebuild, while the question a harness sha must answer is
    // "which measurement procedure produced this row".
    let source = include_str!("transport_receive_ab.rs");
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    // sha2 0.11 returns a generic `Array`, which does not implement `LowerHex`,
    // so the hex rendering is explicit rather than a `{:x}` format.
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn isa_summary() -> String {
    let mut features: Vec<&str> = Vec::new();
    if cfg!(target_feature = "avx2") {
        features.push("avx2");
    }
    if cfg!(target_feature = "avx512f") {
        features.push("avx512f");
    }
    if cfg!(target_feature = "sse4.2") {
        features.push("sse4.2");
    }
    if features.is_empty() {
        features.push("baseline");
    }
    features.join("+")
}

fn main() {
    let payload_bytes: usize = 64 * 1024;
    let receives_per_thread: usize = 400;
    let rounds: usize = 12;
    let threads = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(4)
        .min(8);

    let config = AsupersyncConfig::default();
    let shipped = InMemoryTransport::new();
    shipped
        .send(artifact(payload_bytes), &config)
        .expect("seed shipped transport");
    let legacy = LegacyInMemoryTransport::default();
    legacy.store(artifact(payload_bytes), &config);

    // PARITY BEFORE TIMING. A faster arm that returns something different is not
    // a faster arm. Both must yield byte-identical artifacts, and both must
    // still refuse a request the capability gate denies.
    let a = shipped
        .receive(ARTIFACT_ID, &config)
        .expect("shipped receive");
    let b = legacy
        .receive(ARTIFACT_ID, &config)
        .expect("legacy receive");
    assert_eq!(a.artifact_id, b.artifact_id, "parity: artifact_id");
    assert_eq!(a.source_len, b.source_len, "parity: source_len");
    assert_eq!(a.repair_symbols, b.repair_symbols, "parity: repair_symbols");
    assert_eq!(a.encoded_bytes, b.encoded_bytes, "parity: encoded_bytes");
    let denied = AsupersyncConfig::default().with_capabilities(CapabilitySet::default());
    assert!(
        shipped.receive(ARTIFACT_ID, &denied).is_err(),
        "parity: shipped must still refuse a denied capability set"
    );
    assert!(
        legacy.receive(ARTIFACT_ID, &denied).is_err(),
        "parity: legacy control must refuse the same way"
    );
    assert!(
        shipped.receive("no-such-artifact", &config).is_err(),
        "parity: missing artifact must still error"
    );
    assert!(
        legacy.receive("no-such-artifact", &config).is_err(),
        "parity: legacy control must error the same way"
    );

    // Warm both arms so the first timed round does not pay page-fault and
    // allocator-growth costs that belong to neither.
    for _ in 0..2 {
        run_arm(
            Arm::Shipped,
            &shipped,
            &legacy,
            &config,
            threads,
            receives_per_thread,
        );
        run_arm(
            Arm::Legacy,
            &shipped,
            &legacy,
            &config,
            threads,
            receives_per_thread,
        );
    }

    // BALANCED SQUARE: ABBA on even rounds, BAAB on odd. Every arm occupies every
    // position within each pair of rounds, so monotone drift cancels rather than
    // landing on whichever arm happens to run first.
    let mut shipped_samples: Vec<Duration> = Vec::new();
    let mut legacy_samples: Vec<Duration> = Vec::new();
    let mut null_a: Vec<Duration> = Vec::new();
    let mut null_b: Vec<Duration> = Vec::new();

    for round in 0..rounds {
        let order = if round % 2 == 0 {
            [Arm::Shipped, Arm::Legacy, Arm::Legacy, Arm::Shipped]
        } else {
            [Arm::Legacy, Arm::Shipped, Arm::Shipped, Arm::Legacy]
        };
        for arm in order {
            let elapsed = run_arm(
                arm,
                &shipped,
                &legacy,
                &config,
                threads,
                receives_per_thread,
            );
            match arm {
                Arm::Shipped => shipped_samples.push(elapsed),
                Arm::Legacy => legacy_samples.push(elapsed),
            }
        }

        // A/A NULL, interleaved in the same round and through the identical code
        // path, so it experiences the same machine conditions as the real arms.
        let first = run_arm(
            Arm::Shipped,
            &shipped,
            &legacy,
            &config,
            threads,
            receives_per_thread,
        );
        let second = run_arm(
            Arm::Shipped,
            &shipped,
            &legacy,
            &config,
            threads,
            receives_per_thread,
        );
        null_a.push(first);
        null_b.push(second);
    }

    // SEQUENTIAL NEGATIVE CONTROL: one thread, no contention. The lever only
    // moves work out of a CONTENDED critical section, so this should read ~1.0x.
    let mut seq_shipped: Vec<Duration> = Vec::new();
    let mut seq_legacy: Vec<Duration> = Vec::new();
    for round in 0..rounds {
        let order = if round % 2 == 0 {
            [Arm::Shipped, Arm::Legacy]
        } else {
            [Arm::Legacy, Arm::Shipped]
        };
        for arm in order {
            let elapsed = run_arm(arm, &shipped, &legacy, &config, 1, receives_per_thread);
            match arm {
                Arm::Shipped => seq_shipped.push(elapsed),
                Arm::Legacy => seq_legacy.push(elapsed),
            }
        }
    }

    let shipped_ns = median_ns(shipped_samples);
    let legacy_ns = median_ns(legacy_samples);
    let null_ratio = median_ns(null_b) / median_ns(null_a);
    let seq_ratio = median_ns(seq_legacy) / median_ns(seq_shipped);
    let ratio = legacy_ns / shipped_ns;

    println!("=== br-frankenpandas-3nzz3 InMemoryTransport::receive A/B ===");
    println!("harness            : crates/fp-runtime/examples/transport_receive_ab.rs");
    println!("harness_sha256     : {}", harness_sha256());
    println!(
        "worker             : {}",
        std::env::var("RCH_WORKER").unwrap_or_else(|_| read_trimmed("/proc/sys/kernel/hostname"))
    );
    println!(
        "hostname           : {}",
        read_trimmed("/proc/sys/kernel/hostname")
    );
    println!("logical_threads    : {}", threads);
    println!(
        "available_parallel : {:?}",
        std::thread::available_parallelism().map(std::num::NonZeroUsize::get)
    );
    println!(
        "governor           : {}",
        read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
    );
    println!("isa                : {}", isa_summary());
    println!("profile            : release");
    println!(
        "shape              : payload={payload_bytes}B receives/thread={receives_per_thread} rounds={rounds}"
    );
    println!("design             : balanced square ABBA/BAAB, same invocation, same worker");
    println!("---");
    println!("A/A null ratio     : {null_ratio:.4}x   (must be near 1.0 or the run is void)");
    println!("sequential control : {seq_ratio:.4}x   (1 thread; lever should NOT show here)");
    println!("---");
    println!("shipped  (Arc)     : {:.3} ms median", shipped_ns / 1.0e6);
    println!("legacy   (control) : {:.3} ms median", legacy_ns / 1.0e6);
    println!("RATIO legacy/shipped: {ratio:.4}x   (>1 means the shipped lever is faster)");
}
