#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    assert!(
        fp_conformance::fuzz_index_align_bytes(data).is_ok(),
        "align_union invariants should hold for all projected index inputs"
    );
});
