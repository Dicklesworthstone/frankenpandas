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
//! which is the wrong shape for an internal concurrency comparison.
//!
//! WHAT IS COMPARED
//!
//!   SHIPPED  `fp_runtime::asupersync::transport::InMemoryTransport`
//!            Arc clone under the lock, deep clone (and `catch_unwind`) outside.
//!   LEGACY   `LegacyInMemoryTransport` below — the pre-3nzz3 shape, reproduced
//!            here as a CONTROL. It deep-clones the artifact WHILE HOLDING the
//!            mutex, which is exactly the critical section the lever shortened.
//!
//! The control lives in this harness, never in production code, so the shipped
//! path is measured exactly as it ships.
//!
//! WHY CONCURRENTLY, AND WHY A SEQUENTIAL ROW TOO
//!
//! Both shapes do the same total work per call; only the mutex HOLD TIME
//! differs, so the lever can only pay off under CONTENTION. Threads all pull the
//! same artifact id, which is maximum contention. The 1-thread row is the
//! negative control: the lever should be invisible there, and if it is not, this
//! harness is measuring something other than lock hold time.
//!
//! MEASUREMENT DISCIPLINE (campaign law)
//!
//!   * Both arms run in the SAME invocation on the SAME worker. A ratio built
//!     from two invocations is not evidence — the same cell has read 1.2693x and
//!     0.0093x on two workers with both A/A nulls passing.
//!   * Arms are INTERLEAVED in a balanced square (ABBA / BAAB by round) so drift
//!     cancels instead of loading onto whichever arm ran first.
//!   * The A/A null is run THROUGH THE SAME SQUARE: four runs of the SHIPPED arm
//!     placed in the same four slots, then scored exactly as the real ratio is.
//!     It therefore answers the only question a null should — "if the two arms
//!     were identical, what would this DESIGN report?" A null computed as
//!     first-vs-second instead MEASURES the position effect the square exists to
//!     remove, which is how the first version of this harness produced 0.7966x.
//!   * Threads are spawned ONCE, outside every timed region. Per-call
//!     `thread::scope` put a variable ~400us spawn cost inside a ~20ms
//!     measurement and is a known way to break the null on parallel kernels.
//!   * The row names the WORKER and the HARNESS (this file's SHA-256), because
//!     two separately-sanctioned harnesses have disagreed ~2x on ONE worker with
//!     both nulls green.
//!
//! Run:
//!   RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- \
//!     cargo run --release -p fp-runtime --features asupersync \
//!       --example transport_receive_ab

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
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
/// The ONLY intended difference from the shipped transport is WHERE the deep
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
const ARM_SHIPPED: usize = 0;
const ARM_LEGACY: usize = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arm {
    Shipped,
    Legacy,
}

fn artifact(payload_bytes: usize) -> EncodedArtifact {
    // Size is the whole point: the lever moves a Vec<u8> copy out of the
    // critical section, so any effect scales with how long that copy takes.
    let encoded_bytes: Vec<u8> = (0..payload_bytes).map(|i| (i % 251) as u8).collect();
    EncodedArtifact {
        artifact_id: ARTIFACT_ID.to_string(),
        source_len: payload_bytes,
        encoded_bytes,
        repair_symbols: 1,
    }
}

/// Shared control block for the once-spawned worker pool.
struct Control {
    start: Barrier,
    finish: Barrier,
    arm: AtomicUsize,
    active: AtomicUsize,
    receives: AtomicUsize,
    stop: AtomicBool,
}

/// Time ONE unit: `active` workers each performing `receives` receives on `arm`.
/// The timed span is barrier-to-barrier and contains no thread management.
fn time_unit(control: &Control, arm: Arm, active: usize, receives: usize) -> Duration {
    control.arm.store(
        match arm {
            Arm::Shipped => ARM_SHIPPED,
            Arm::Legacy => ARM_LEGACY,
        },
        Ordering::SeqCst,
    );
    control.active.store(active, Ordering::SeqCst);
    control.receives.store(receives, Ordering::SeqCst);
    let started = Instant::now();
    control.start.wait();
    control.finish.wait();
    started.elapsed()
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
    // Hash THIS SOURCE FILE, not the binary: a binary hash moves with every
    // dependency rebuild, while the question a harness sha must answer is
    // "which measurement PROCEDURE produced this row".
    let mut hasher = Sha256::new();
    hasher.update(include_str!("transport_receive_ab.rs").as_bytes());
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

/// Score four slot timings the way the ABBA square scores the real arms.
/// Slots 0 and 3 belong to the arm that led the round; slots 1 and 2 to the
/// other. Returns (leader_ns, follower_ns).
fn score_square(slots: &[Duration; 4]) -> (f64, f64) {
    (
        median_ns(vec![slots[0], slots[3]]),
        median_ns(vec![slots[1], slots[2]]),
    )
}

fn main() {
    let payload_bytes: usize = 64 * 1024;
    // Sized for signal, not for wall-clock: on rch nearly all of this example's
    // runtime is COMPILING the asupersync dependency tree, so the measurement
    // phase is effectively free and there is no reason to under-sample. At 400
    // receives x 16 rounds the A/A null came in at 1.0853x on vmi1152480, which
    // is too loose to sit under a 1.8x claim on a busy shared host.
    let receives_per_thread: usize = 2_000;
    let rounds: usize = 32;
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
    // a faster arm, and the bead requires the error surfaces to be preserved.
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
        "parity: shipped must refuse a denied capability set"
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

    let control = Control {
        start: Barrier::new(threads + 1),
        finish: Barrier::new(threads + 1),
        arm: AtomicUsize::new(ARM_SHIPPED),
        active: AtomicUsize::new(threads),
        receives: AtomicUsize::new(receives_per_thread),
        stop: AtomicBool::new(false),
    };

    let mut ab_shipped: Vec<Duration> = Vec::new();
    let mut ab_legacy: Vec<Duration> = Vec::new();
    let mut null_leader: Vec<Duration> = Vec::new();
    let mut null_follower: Vec<Duration> = Vec::new();
    let mut seq_shipped: Vec<Duration> = Vec::new();
    let mut seq_legacy: Vec<Duration> = Vec::new();

    std::thread::scope(|scope| {
        for worker in 0..threads {
            let control = &control;
            let shipped = &shipped;
            let legacy = &legacy;
            let config = &config;
            scope.spawn(move || {
                loop {
                    control.start.wait();
                    if control.stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if worker < control.active.load(Ordering::SeqCst) {
                        let arm = control.arm.load(Ordering::SeqCst);
                        let receives = control.receives.load(Ordering::SeqCst);
                        let mut checksum: u64 = 0;
                        for _ in 0..receives {
                            let got = if arm == ARM_SHIPPED {
                                shipped.receive(ARTIFACT_ID, config)
                            } else {
                                legacy.receive(ARTIFACT_ID, config)
                            }
                            .expect("receive must succeed");
                            // Consume the result so neither arm can be optimized
                            // away, and both pay the same read cost.
                            checksum = checksum
                                .wrapping_add(u64::from(got.encoded_bytes[0]))
                                .wrapping_add(u64::from(
                                    got.encoded_bytes[got.encoded_bytes.len() - 1],
                                ))
                                .wrapping_add(got.source_len as u64);
                        }
                        std::hint::black_box(checksum);
                    }
                    control.finish.wait();
                }
            });
        }

        // Warm both arms so the first timed round does not pay page-fault and
        // allocator-growth costs that belong to neither.
        for _ in 0..3 {
            time_unit(&control, Arm::Shipped, threads, receives_per_thread);
            time_unit(&control, Arm::Legacy, threads, receives_per_thread);
        }

        for round in 0..rounds {
            // BALANCED SQUARE: the leader alternates, so each arm occupies each
            // slot equally across any pair of rounds.
            let (lead, follow) = if round % 2 == 0 {
                (Arm::Shipped, Arm::Legacy)
            } else {
                (Arm::Legacy, Arm::Shipped)
            };
            let order = [lead, follow, follow, lead];
            let mut slots = [Duration::ZERO; 4];
            for (slot, arm) in order.iter().enumerate() {
                slots[slot] = time_unit(&control, *arm, threads, receives_per_thread);
            }
            let (lead_ns, follow_ns) = score_square(&slots);
            let lead_dur = Duration::from_nanos(lead_ns as u64);
            let follow_dur = Duration::from_nanos(follow_ns as u64);
            match lead {
                Arm::Shipped => {
                    ab_shipped.push(lead_dur);
                    ab_legacy.push(follow_dur);
                }
                Arm::Legacy => {
                    ab_legacy.push(lead_dur);
                    ab_shipped.push(follow_dur);
                }
            }

            // A/A NULL THROUGH THE IDENTICAL SQUARE: four runs of the SHIPPED
            // arm, scored exactly as above. This asks what the DESIGN reports
            // when the two arms are the same code, which is the only thing a
            // null can legitimately certify.
            let mut null_slots = [Duration::ZERO; 4];
            for slot in &mut null_slots {
                *slot = time_unit(&control, Arm::Shipped, threads, receives_per_thread);
            }
            let (null_lead_ns, null_follow_ns) = score_square(&null_slots);
            null_leader.push(Duration::from_nanos(null_lead_ns as u64));
            null_follower.push(Duration::from_nanos(null_follow_ns as u64));

            // SEQUENTIAL NEGATIVE CONTROL: one worker, no contention, same
            // square. The lever only moves work out of a CONTENDED critical
            // section, so this should land near 1.0.
            let seq_order = [lead, follow, follow, lead];
            let mut seq_slots = [Duration::ZERO; 4];
            for (slot, arm) in seq_order.iter().enumerate() {
                seq_slots[slot] = time_unit(&control, *arm, 1, receives_per_thread);
            }
            let (seq_lead_ns, seq_follow_ns) = score_square(&seq_slots);
            let seq_lead = Duration::from_nanos(seq_lead_ns as u64);
            let seq_follow = Duration::from_nanos(seq_follow_ns as u64);
            match lead {
                Arm::Shipped => {
                    seq_shipped.push(seq_lead);
                    seq_legacy.push(seq_follow);
                }
                Arm::Legacy => {
                    seq_legacy.push(seq_lead);
                    seq_shipped.push(seq_follow);
                }
            }
        }

        control.stop.store(true, Ordering::SeqCst);
        control.start.wait();
    });

    let shipped_ns = median_ns(ab_shipped);
    let legacy_ns = median_ns(ab_legacy);
    let null_ratio = median_ns(null_follower) / median_ns(null_leader);
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
    println!("threads_used       : {threads}");
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
    println!("design             : balanced square ABBA/BAAB, pool spawned once, same invocation");
    println!("---");
    println!("A/A null ratio     : {null_ratio:.4}x   (same code both sides; must be near 1.0)");
    println!("sequential control : {seq_ratio:.4}x   (1 thread; lever should NOT show here)");
    println!("---");
    println!("shipped  (Arc)     : {:.3} ms median", shipped_ns / 1.0e6);
    println!("legacy   (control) : {:.3} ms median", legacy_ns / 1.0e6);
    println!("RATIO legacy/shipped: {ratio:.4}x   (>1 means the shipped lever is faster)");
}
