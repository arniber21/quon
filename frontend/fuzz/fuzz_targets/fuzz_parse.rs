#![no_main]
// Lexing then parsing arbitrary text must never panic — only Ok/Err.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(src) = std::str::from_utf8(data) {
        if let Ok(tokens) = frontend::lexer::lex(src) {
            let _ = frontend::parser::parse(&tokens);
        }
    }
});
