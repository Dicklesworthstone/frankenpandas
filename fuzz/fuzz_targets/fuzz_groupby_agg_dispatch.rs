#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    assert!(
        fp_conformance::fuzz_groupby_agg_bytes(data).is_ok(),
        "groupby agg dispatch invariants should hold for all projected inputs"
    );
});
