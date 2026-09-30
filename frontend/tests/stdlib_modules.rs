//! Typecheck the experimental circuit stdlib and one concrete instantiation
//! of each parametric module (issue #195).

use frontend::check_program;

fn accept(src: &str) {
    match check_program(src) {
        Ok(()) => {}
        Err(diags) => panic!("expected the program to typecheck, got {diags:?}"),
    }
}

fn with_module(module: &str, caller: &str) -> String {
    let mut src = String::from(module);
    if !src.ends_with('\n') {
        src.push('\n');
    }
    src.push_str(caller);
    src
}

#[test]
fn stdlib_modules_typecheck_alone() {
    accept(include_str!("../../stdlib/qft.qn"));
    accept(include_str!("../../stdlib/oracles.qn"));
    accept(include_str!("../../stdlib/qaoa.qn"));
    accept(include_str!("../../stdlib/ising.qn"));
    accept(include_str!("../../stdlib/phase_estimation.qn"));
}

#[test]
fn stdlib_parametric_instantiations_typecheck() {
    accept(&with_module(
        include_str!("../../stdlib/qft.qn"),
        "fn use_qft(): Circuit<3, 3, 18, Universal> = qft(3)\n\
         fn use_roundtrip(): Circuit<2, 2, 16, Universal> = qft_roundtrip(2)\n",
    ));
    accept(&with_module(
        include_str!("../../stdlib/qaoa.qn"),
        "fn use_qaoa(): Circuit<3, 3, 10, Universal> = qaoa_layer(3, 0.5, 0.1)\n",
    ));
    accept(&with_module(
        include_str!("../../stdlib/ising.qn"),
        "fn use_ising(): Q<List<Bit>> = run {\n\
             q <- ising_evolve(4, 1.0, 1.0, 0.0, 3) @ qreg(4)\n\
             measure_all(q)\n\
         }\n",
    ));
    accept(&with_module(
        include_str!("../../stdlib/phase_estimation.qn"),
        "fn use_qpe(): Circuit<2, 2, 4, Clifford> = qpe_1bit()\n",
    ));
    accept(&with_module(
        include_str!("../../stdlib/oracles.qn"),
        "fn use_bv(): Circuit<4, 4, 2, Clifford> = bv_oracle_101()\n\
         fn use_mark(): Circuit<2, 2, 1, Clifford> = grover_mark_11()\n",
    ));
}
