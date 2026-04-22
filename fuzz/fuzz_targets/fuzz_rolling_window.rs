#![no_main]

use libfuzzer_sys::fuzz_target;

const MAX_ROLLING_WINDOW_FUZZ_BYTES: usize = 256 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_ROLLING_WINDOW_FUZZ_BYTES {
        return;
    }

    let _ = fp_conformance::fuzz_rolling_window_bytes(data);
});
