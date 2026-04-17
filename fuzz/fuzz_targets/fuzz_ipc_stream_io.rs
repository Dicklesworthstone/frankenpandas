#![no_main]

use libfuzzer_sys::fuzz_target;

const MAX_IPC_STREAM_BYTES: usize = 512 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_IPC_STREAM_BYTES {
        return;
    }

    let _ = fp_conformance::fuzz_ipc_stream_io_bytes(data);
});
