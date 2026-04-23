#![no_main]

use libfuzzer_sys::fuzz_target;

const MAX_PARSE_EXPR_BYTES: usize = 4 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_PARSE_EXPR_BYTES {
        return;
    }

    let _ = fp_conformance::fuzz_parse_expr_bytes(data);
});
