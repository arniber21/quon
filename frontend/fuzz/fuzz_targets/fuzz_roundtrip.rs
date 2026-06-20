#![no_main]
// The centerpiece: generate a syntactically valid program, print it, and check that
// lexing + parsing + reprinting reproduces byte-identical source. Because the printer fully
// parenthesizes, stable printed output implies a stable AST — any precedence/associativity
// or desugaring asymmetry between parser and printer is caught here.

use arbitrary::Unstructured;
use frontend::lexer::lex;
use frontend::parser::parse;
use frontend::pretty::pretty;
use frontend_fuzz::gen::arb_program;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok(decls) = arb_program(&mut u) else {
        return;
    };

    let printed = pretty(&decls);
    let tokens = lex(&printed).unwrap_or_else(|e| {
        panic!("generated program failed to lex: {e:?}\n--- source ---\n{printed}")
    });
    let reparsed = parse(&tokens).unwrap_or_else(|e| {
        panic!("generated program failed to parse: {e:?}\n--- source ---\n{printed}")
    });
    let printed2 = pretty(&reparsed);
    assert_eq!(
        printed, printed2,
        "roundtrip changed the printed form\n--- first ---\n{printed}\n--- second ---\n{printed2}"
    );
});
