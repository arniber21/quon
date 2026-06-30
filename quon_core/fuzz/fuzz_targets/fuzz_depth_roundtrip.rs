#![no_main]

use libfuzzer_sys::fuzz_target;
use quon_core::DepthExpr;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(expr) = DepthExpr::parse(s) {
            let _ = DepthExpr::parse(&expr.to_sexpr());
        }
    }
});
