#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    assert!(
        fp_conformance::fuzz_common_dtype_bytes(data).is_ok(),
        "common_dtype invariants should hold for all projected dtype pairs"
    );
});
