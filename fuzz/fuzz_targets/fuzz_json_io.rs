#![no_main]

use libfuzzer_sys::fuzz_target;

const MAX_JSON_IO_BYTES: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_JSON_IO_BYTES {
        return;
    }

    let _ = fp_conformance::fuzz_json_io_bytes(data);
});
