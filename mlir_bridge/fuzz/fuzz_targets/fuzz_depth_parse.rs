//! Fuzz `DepthExpr::parse`: arbitrary bytes must always yield `Ok`/`Err`, never
//! a panic. Run with `cargo +nightly fuzz run fuzz_depth_parse`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mlir_bridge::dialect::depth::DepthExpr;

fuzz_target!(|data: &[u8]| {
    if let Ok(source) = std::str::from_utf8(data) {
        let _ = DepthExpr::parse(source);
    }
});
