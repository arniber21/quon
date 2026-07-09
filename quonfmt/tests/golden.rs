mod support;

use quonfmt::format_str;

macro_rules! golden {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() {
            let input = include_str!(concat!("corpus/input/", $file));
            let formatted = format_str(input).expect("parse");
            insta::assert_snapshot!(stringify!($name), formatted);
            support::assert_ast_stable(input, &formatted);
        }
    };
}

golden!(golden_decls, "decls.qn");
golden!(golden_circuit_compose, "circuit_compose.qn");
golden!(golden_run_binds, "run_binds.qn");
golden!(golden_borrow, "borrow.qn");
golden!(golden_types, "types.qn");
golden!(golden_expr_precedence, "expr_precedence.qn");
golden!(golden_par_repeat, "par_repeat.qn");
golden!(golden_match_if, "match_if.qn");
golden!(golden_lambdas, "lambdas.qn");
golden!(golden_application, "application.qn");

#[test]
fn golden_frontend_fixtures() {
    let fixtures = [
        "bell_state.qn",
        "bernstein_vazirani.qn",
        "error_correction.qn",
        "grover.qn",
        "ising.qn",
        "qaoa.qn",
        "shor.qn",
        "stdlib_forms.qn",
        "teleport.qn",
    ];
    for name in fixtures {
        let input = std::fs::read_to_string(format!(
            "{}/../frontend/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|_| panic!("read fixture {name}"));
        let formatted = format_str(&input).unwrap_or_else(|_| panic!("format fixture {name}"));
        insta::assert_snapshot!(name, formatted);
        support::assert_ast_stable(&input, &formatted);
    }
}
