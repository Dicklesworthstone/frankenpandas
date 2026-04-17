#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    assert!(
        fp_conformance::fuzz_groupby_sum_bytes(data).is_ok(),
        "groupby_sum invariants should hold for all projected key/value series inputs"
    );
});
