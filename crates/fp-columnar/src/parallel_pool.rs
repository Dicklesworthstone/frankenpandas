//! ⚠️ MEASURED REJECT — NOT WIRED IN. Retained as evidence, not as code.
//!
//! This module is deliberately NOT declared with `mod parallel_pool;`, so it
//! compiles into nothing. It exists so the next person who reaches the same
//! conclusion I did finds the measurement instead of rebuilding it.
//!
//! THE PREMISE WAS WRONG. `df.dot` was believed to pay a per-call
//! `std::thread::scope` fan-out. It does not: `fp_frame::str_worker_pool` is
//! already a process-wide persistent pool with per-worker channels and
//! `'static` jobs, and `DataFrame::dot` already dispatches through it
//! (fp-frame/src/lib.rs, `str_worker_pool::pool().run(jobs)`). This module is
//! an independent reimplementation of that same design.
//!
//! WHAT WAS MEASURED, in order:
//!   * Amdahl fit over `df_dot @100k`: residual FLAT at ~931us across W=8 and
//!     W=32. A per-worker spawn cost would scale with W. First refutation.
//!   * Direct probe, no-op fan-out, best of 200: `thread::scope` 203/592/1011us
//!     at W=8/32/63 against this pool's 19/50/98us. This LOOKS like a 542us
//!     prize at W=32 and is what justified building the module — it overstates
//!     the real cost, because with live workers thread creation overlaps work
//!     already running.
//!   * A TREE fan-out (scoped threads spawning scoped threads, so dispatch is
//!     itself parallel) was built and rejected: 615us at W=32 and 1650us at
//!     W=63 — no better, worse at the top. Thread creation is kernel work and
//!     contends whoever issues it.
//!   * Wired into `ScalarValues::materialize_dot_columns_with_policy` and
//!     measured: base median 901.8us vs pooled 937.4us over three interleaved
//!     passes. NO GAIN.
//!   * NON-VACUITY WITNESS, which is the actual finding: `perf` showed 63
//!     `fp-str-N` threads and ZERO `fp-pool-N`. The lever never executed. The
//!     path it was wired into is a secondary one this lane never reaches,
//!     because the fp-frame dispatch has materialized every column first.
//!
//! No vs-pandas ratio was banked for it: a lever that provably never ran would
//! be measuring nothing, and a row for it would be fabricated.
//!
//! DO NOT WIRE THIS IN without first showing, by thread name, that the target
//! call site is not already served by `fp_frame::str_worker_pool`.

//! Persistent worker pool for the column-parallel kernels.
//!
//! br-frankenpandas-oarkz / 633fb. `std::thread::scope` creates and destroys its
//! threads on EVERY call, and that is not free at the widths this crate uses.
//! Measured on thinkstation1 (64 logical CPUs), one fan-out of no-op workers,
//! best of 200:
//!
//! | workers            |   8 |  16 |  32 |   63 |
//! |--------------------|-----|-----|-----|------|
//! | `thread::scope` us | 203 | 329 | 592 | 1011 |
//! | this pool       us |  19 |  32 |  50 |   98 |
//!
//! ⚠️ THAT TABLE IS A NO-OP FAN-OUT AND OVERSTATES THE LIVE COST. It was read
//! as "the scope fan-out is a large fraction of `df_dot @100k`'s ~1018us"; that
//! reading is REFUTED — see the reject header above. With live workers, thread
//! creation overlaps work already running, and this lane does not use a scope
//! at all. A TREE fan-out (scoped threads spawning scoped threads, so the
//! dispatch is itself parallel) was also measured and REJECTED: 615us at 32 and
//! 1650us at 63, i.e. no better and worse at the top. Thread creation is kernel
//! work and contends regardless of who issues it.
//!
//! ⚠️ JOBS ARE `'static`, DELIBERATELY. A pool that accepts borrowed work needs
//! lifetime erasure, which is `unsafe`, and this crate is `forbid(unsafe_code)`.
//! Callers therefore hand over OWNED payloads (clone the `Arc`s) and get owned
//! results back; any write-back into borrowed state happens on the calling
//! thread after [`run`] returns. That is why `run` returns results rather than
//! letting workers publish.

use std::sync::mpsc;
use std::sync::OnceLock;

type Job = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    /// One channel per worker rather than one shared `Mutex<Receiver>`: a shared
    /// receiver forces every worker to contend for the same lock, and the worker
    /// that wins it BLOCKS inside `recv` while still holding it. Per-worker
    /// channels are what the measurement above was taken with.
    senders: Vec<mpsc::Sender<Job>>,
}

static POOL: OnceLock<Pool> = OnceLock::new();

fn pool() -> &'static Pool {
    POOL.get_or_init(|| {
        let workers = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1);
        let mut senders = Vec::with_capacity(workers);
        for index in 0..workers {
            let (tx, rx) = mpsc::channel::<Job>();
            let spawned = std::thread::Builder::new()
                .name(format!("fp-pool-{index}"))
                .spawn(move || {
                    // Ends when the pool's sender is dropped, which only happens
                    // at process teardown since `POOL` is a static.
                    while let Ok(job) = rx.recv() {
                        job();
                    }
                });
            if spawned.is_ok() {
                senders.push(tx);
            }
            // A thread that will not spawn is not fatal: the pool simply ends up
            // narrower, and `run` falls back to the caller's thread at width 0.
        }
        Pool { senders }
    })
}

/// Worker threads actually alive. `0` means every spawn failed and [`run`]
/// executes inline.
#[must_use]
pub fn worker_count() -> usize {
    pool().senders.len()
}

/// Run `jobs` on the persistent pool, returning their results IN SUBMISSION
/// ORDER.
///
/// Blocks until every job has finished, so a caller may safely rely on all
/// borrowed state it captured by value being finished with on return.
///
/// A single job runs inline: dispatching one unit of work costs a send, a
/// receive and a wakeup to save nothing.
pub fn run<T: Send + 'static>(jobs: Vec<Box<dyn FnOnce() -> T + Send + 'static>>) -> Vec<T> {
    let count = jobs.len();
    if count == 0 {
        return Vec::new();
    }
    let pool = pool();
    let workers = pool.senders.len();
    if count == 1 || workers == 0 {
        return jobs.into_iter().map(|job| job()).collect();
    }

    let (result_tx, result_rx) = mpsc::channel::<(usize, T)>();
    let mut dispatched = 0usize;
    for (index, job) in jobs.into_iter().enumerate() {
        let result_tx = result_tx.clone();
        let wrapped: Job = Box::new(move || {
            // A closed result channel means the caller is gone; the value is
            // simply dropped. It cannot happen while `run` is blocked below.
            let _ = result_tx.send((index, job()));
        });
        match pool.senders[index % workers].send(wrapped) {
            Ok(()) => dispatched += 1,
            Err(mpsc::SendError(returned)) => {
                // That worker died. Run its job here so the caller still gets a
                // complete answer rather than a hang.
                returned();
                dispatched += 1;
            }
        }
    }
    drop(result_tx);

    let mut slots: Vec<Option<T>> = Vec::with_capacity(count);
    slots.resize_with(count, || None);
    for _ in 0..dispatched {
        match result_rx.recv() {
            Ok((index, value)) => slots[index] = Some(value),
            // Every sender dropped without a value: nothing more is coming.
            Err(mpsc::RecvError) => break,
        }
    }
    slots
        .into_iter()
        .map(|slot| slot.expect("every dispatched job reports exactly one result"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_preserves_submission_order_regardless_of_completion_order() {
        // Later jobs finish FIRST, so a pool that returned completion order
        // would scramble these. Results are addressed by index, not by arrival.
        let jobs: Vec<Box<dyn FnOnce() -> usize + Send + 'static>> = (0..64usize)
            .map(|i| {
                Box::new(move || {
                    let spin = (64 - i) * 20_000;
                    let mut acc = 0usize;
                    for x in 0..spin {
                        acc = acc.wrapping_add(x);
                    }
                    std::hint::black_box(acc);
                    i
                }) as Box<dyn FnOnce() -> usize + Send + 'static>
            })
            .collect();
        assert_eq!(run(jobs), (0..64usize).collect::<Vec<_>>());
    }

    #[test]
    fn run_handles_empty_single_and_repeated_dispatch() {
        assert!(run::<usize>(Vec::new()).is_empty());
        let one: Vec<Box<dyn FnOnce() -> usize + Send + 'static>> = vec![Box::new(|| 7)];
        assert_eq!(run(one), vec![7]);
        // The pool is a static: a second dispatch must reuse the SAME threads,
        // which is the entire point. Repeat enough to catch a pool that quietly
        // leaks a thread per call.
        for round in 0..50usize {
            let jobs: Vec<Box<dyn FnOnce() -> usize + Send + 'static>> = (0..8usize)
                .map(|i| Box::new(move || i * round) as Box<dyn FnOnce() -> usize + Send + 'static>)
                .collect();
            assert_eq!(
                run(jobs),
                (0..8usize).map(|i| i * round).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn worker_count_is_positive_on_a_host_that_can_spawn() {
        assert!(worker_count() >= 1, "pool must have at least one worker");
    }
}
