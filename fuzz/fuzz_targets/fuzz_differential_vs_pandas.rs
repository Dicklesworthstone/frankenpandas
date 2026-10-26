#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    assert!(
        fp_conformance::fuzz_differential_vs_pandas_bytes(data).is_ok(),
        "differential fuzzer found divergence against live pandas 2.2.3 oracle"
    );
});
