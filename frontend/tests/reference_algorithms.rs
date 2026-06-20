// The 8 reference algorithms (SPEC §12) must lex and parse (issue #7 headline criterion),
// and their parsed ASTs are locked with insta snapshots for regression.

mod support;

use frontend::lexer::lex;
use frontend::parser::parse;
use support::strip_decls;

fn parse_src(name: &str, src: &str) -> Vec<frontend::lexer::Sp<frontend::ast::Decl>> {
    let tokens = lex(src).unwrap_or_else(|errs| {
        for (msg, span) in &errs {
            eprintln!("{name}: lex error at {span}: {msg}");
        }
        panic!("{name}: lexing failed");
    });
    let mut decls = parse(&tokens).unwrap_or_else(|errs| {
        for (msg, span) in &errs {
            let around = src.get(span.start..span.end.min(src.len())).unwrap_or("");
            eprintln!("{name}: parse error at {span} (near {around:?}): {msg}");
        }
        panic!("{name}: parsing failed");
    });
    strip_decls(&mut decls);
    decls
}

macro_rules! fixture_test {
    ($test:ident, $file:literal) => {
        #[test]
        fn $test() {
            let src = include_str!(concat!("fixtures/", $file));
            let decls = parse_src($file, src);
            assert!(
                !decls.is_empty(),
                "{}: expected at least one declaration",
                $file
            );
            // Snapshot the canonical pretty-printed source rather than the raw AST debug:
            // readable, stable, and easy to review long-term (and it doubles as a printer test).
            insta::assert_snapshot!(stringify!($test), frontend::pretty::pretty(&decls));
        }
    };
}

fixture_test!(bell_state, "bell_state.qn");
fixture_test!(teleport, "teleport.qn");
fixture_test!(grover, "grover.qn");
fixture_test!(shor, "shor.qn");
fixture_test!(error_correction, "error_correction.qn");
fixture_test!(qaoa, "qaoa.qn");
fixture_test!(bernstein_vazirani, "bernstein_vazirani.qn");
fixture_test!(ising, "ising.qn");
fixture_test!(stdlib_forms, "stdlib_forms.qn");
