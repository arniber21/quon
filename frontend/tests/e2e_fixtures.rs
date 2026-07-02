//! End-to-end pipeline smoke: parse → desugar → typecheck on every reference fixture.

use frontend::{desugar, parse_program, typecheck::TypeChecker};

fn check_program(name: &str, src: &str) {
    let decls = parse_program(src).unwrap_or_else(|e| panic!("{name}: parse: {e:?}"));
    let decls = desugar::desugar_decls(decls).unwrap_or_else(|e| panic!("{name}: desugar: {e:?}"));
    TypeChecker::new()
        .check_decls(&decls)
        .unwrap_or_else(|e| panic!("{name}: typecheck: {e:?}"));
}

macro_rules! e2e_fixture {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() {
            let src = include_str!(concat!("fixtures/", $file));
            check_program($file, src);
        }
    };
}

e2e_fixture!(e2e_bell_state, "bell_state.qn");
e2e_fixture!(e2e_grover, "grover.qn");
e2e_fixture!(e2e_shor, "shor.qn");
e2e_fixture!(e2e_error_correction, "error_correction.qn");
e2e_fixture!(e2e_qaoa, "qaoa.qn");
e2e_fixture!(e2e_bernstein_vazirani, "bernstein_vazirani.qn");
e2e_fixture!(e2e_ising, "ising.qn");
e2e_fixture!(e2e_stdlib_forms, "stdlib_forms.qn");

#[test]
fn e2e_teleport() {
    // `teleport.qn` references `bell_state`; load both like the typecheck suite.
    let src = concat!(
        include_str!("fixtures/bell_state.qn"),
        "\n",
        include_str!("fixtures/teleport.qn"),
    );
    check_program("teleport.qn", src);
}

#[test]
fn e2e_verify_harness_fixtures_exist() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../test/verify");
    for stem in ["bell", "teleport", "bernstein_vazirani"] {
        let qn = root.join(format!("{stem}.qn"));
        let py = root.join(format!("{stem}.py"));
        assert!(qn.is_file(), "missing {}", qn.display());
        assert!(py.is_file(), "missing {}", py.display());
    }
}
