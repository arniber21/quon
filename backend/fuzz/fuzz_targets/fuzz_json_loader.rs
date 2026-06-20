//! Fuzz the untrusted-input JSON loader: arbitrary bytes must always yield
//! `Ok`/`Err`, never a panic. Run with `cargo +nightly fuzz run fuzz_json_loader`.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(src) = std::str::from_utf8(data) {
        let _ = backend::json::from_str(src);
    }
});
