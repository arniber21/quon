#![no_main]
// The lexer must never panic on arbitrary input — it returns Ok(tokens) or Err(spans).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(src) = std::str::from_utf8(data) {
        let _ = frontend::lexer::lex(src);
    }
});
