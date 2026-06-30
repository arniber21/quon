#![no_main]

use libfuzzer_sys::fuzz_target;
use quon_core::classify_use_count;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let count = (data[0] as usize) % 32;
    let has_measure = data.len() > 1 && data[1] % 2 == 0;
    let has_other = data.len() > 2 && data[2] % 2 == 0;
    let _ = classify_use_count(count, has_measure, has_other);
});
