// The pretty-printer must be roundtrip-faithful: parsing its output reproduces the same AST
// (up to spans). Validated here against the SPEC §12 reference algorithms.

mod support;

use frontend::lexer::lex;
use frontend::parser::parse;
use frontend::pretty::pretty;
use support::{parse_stripped, strip_decls};

fn roundtrip(name: &str, src: &str) {
    let original = parse_stripped(src);
    let printed = pretty(&original);
    let tokens = lex(&printed).unwrap_or_else(|e| {
        panic!("{name}: re-lex of printed output failed: {e:?}\n---\n{printed}")
    });
    let mut reparsed = parse(&tokens).unwrap_or_else(|e| {
        panic!("{name}: re-parse of printed output failed: {e:?}\n---\n{printed}")
    });
    strip_decls(&mut reparsed);
    assert_eq!(
        reparsed, original,
        "{name}: pretty-print roundtrip changed the AST\n--- printed ---\n{printed}"
    );
}

macro_rules! roundtrip_test {
    ($test:ident, $file:literal) => {
        #[test]
        fn $test() {
            roundtrip($file, include_str!(concat!("fixtures/", $file)));
        }
    };
}

roundtrip_test!(bell_state, "bell_state.qn");
roundtrip_test!(teleport, "teleport.qn");
roundtrip_test!(grover, "grover.qn");
roundtrip_test!(shor, "shor.qn");
roundtrip_test!(error_correction, "error_correction.qn");
roundtrip_test!(qaoa, "qaoa.qn");
roundtrip_test!(bernstein_vazirani, "bernstein_vazirani.qn");
roundtrip_test!(ising, "ising.qn");
roundtrip_test!(stdlib_forms, "stdlib_forms.qn");
