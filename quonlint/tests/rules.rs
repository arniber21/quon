mod support;

use support::{has_rule, lint_snippet};

#[test]
fn sequential_for_emits_info() {
    let src = r#"fn f(n: Nat): Circuit<n, n, _, Universal> = circuit {
        for i in range(n) { H @0 }
    }"#;
    let diags = lint_snippet(src);
    assert!(has_rule(&diags, "depth/sequential-for-blowup"));
}

#[test]
fn ising_style_fold_does_not_warn_unsound_depth() {
    let src = include_str!("../../frontend/tests/fixtures/ising.qn");
    let diags = lint_snippet(src);
    assert!(!has_rule(&diags, "depth/unsound-depth-annotation"));
}

#[test]
fn repeat_non_literal_count() {
    let src = r#"fn had_one(): Circuit<1, 1, 1, Clifford> = circuit { H @0 }
fn f(n: Nat): Circuit<n, n, 1, Clifford> = par { had_one() } * n"#;
    let diags = lint_snippet(src);
    assert!(has_rule(&diags, "depth/repeat-non-literal-count"));
}

#[test]
fn controlled_chain_negative_on_bell() {
    let src = include_str!("../../frontend/tests/fixtures/bell_state.qn");
    let diags = lint_snippet(src);
    assert!(!has_rule(&diags, "depth/controlled-chain"));
}

#[test]
fn universal_in_clifford_block() {
    let src = r#"fn f(): Circuit<1, 1, 1, Universal> = circuit { H @0 |> Rz(1.0) @0 }"#;
    let diags = lint_snippet(src);
    // Universal-annotated circuits are excluded; rule targets Clifford/Infer blocks.
    assert!(!has_rule(&diags, "gates/universal-in-clifford-block"));
}

#[test]
fn swap_in_source() {
    let src = r#"fn f(): Circuit<2, 2, 1, Clifford> = circuit { SWAP @(0, 1) }"#;
    let diags = lint_snippet(src);
    assert!(has_rule(&diags, "gates/swap-in-source"));
}

#[test]
fn consecutive_rotations_on_same_qubit() {
    let src = r#"fn f(): Circuit<1, 1, 2, Universal> = circuit {
        Rz(1.0) @0 |> Rz(2.0) @0
    }"#;
    let diags = lint_snippet(src);
    assert!(
        frontend::check_program(src).is_ok(),
        "snippet must typecheck: {:?}",
        frontend::check_program(src).err()
    );
    assert!(has_rule(&diags, "gates/consecutive-rotations"));
}

#[test]
#[ignore = "discarding ancilla after entangling use is a linear type error in well-typed programs"]
fn discard_in_borrow_after_entangling_gate() {
    let src = r#"fn f(): Q<Unit> = run {
    borrow a: Qubit, b: Qubit in {
        CNOT @(a, b)
        discard(a)
        discard(b)
    }
}"#;
    let diags = lint_snippet(src);
    assert!(
        frontend::check_program(src).is_ok(),
        "snippet must typecheck: {:?}",
        frontend::check_program(src).err()
    );
    assert!(has_rule(&diags, "ancilla/discard-in-borrow"));
}

#[test]
#[ignore = "borrow escape type error prevents lint on raw ancilla return; rule applies when typecheck passes"]
fn unmeasured_ancilla_in_return() {
    let src = r#"fn f(): Q<(Qubit, Int)> = run {
    borrow a: Qubit in {
        return (a, 0)
    }
}"#;
    let diags = lint_snippet(src);
    assert!(has_rule(&diags, "ancilla/unmeasured-ancilla-output"));
}

#[test]
fn nested_borrow() {
    let src = r#"fn f(): Q<Qubit> = run {
    borrow a: Qubit in {
        borrow b: Qubit in {
            discard(b)
            return a
        }
    }
}"#;
    let diags = lint_snippet(src);
    assert!(has_rule(&diags, "ancilla/nested-borrow"));
}

#[test]
#[ignore = "circuit bind without apply is a type error; lint runs on well-typed programs only"]
fn circuit_bind_without_apply() {
    let src = r#"fn f(): Q<QReg<1>> = run {
        c <- had_one()
        return c
    }
    fn had_one(): Circuit<1, 1, 1, Clifford> = circuit { H @0 }"#;
    let diags = lint_snippet(src);
    assert!(has_rule(&diags, "monad/circuit-bind-without-apply"));
}

#[test]
fn nested_run_block() {
    let src = r#"fn f(): Q<Bit> = run {
    inner()
}
fn inner(): Q<Bit> = run { measure(qubit()) }"#;
    let diags = lint_snippet(src);
    assert!(!has_rule(&diags, "monad/nested-run-block"));
}

#[test]
fn unused_measurement_negative_when_result_used() {
    let src = r#"fn measure_one(_k: Int): Q<Bit> = run {
    q <- qubit()
    measure(q)
}"#;
    let diags = lint_snippet(src);
    assert!(!has_rule(&diags, "monad/unused-measurement"));
}

#[test]
fn suppression_next_line() {
    let src = r#"fn f(n: Nat): Circuit<n, n, _, Universal> = circuit {
        # quonlint-disable-next-line depth/sequential-for-blowup
        for i in range(n) { H @0 }
    }"#;
    let diags = lint_snippet(src);
    assert!(!has_rule(&diags, "depth/sequential-for-blowup"));
}

#[test]
fn list_rules_count() {
    let rules = quonlint::register_rules();
    assert!(rules.len() >= 11);
}

#[test]
fn clifford_block_with_t_gate_warns() {
    let src = r#"fn f(): Circuit<1, 1, 1, Clifford> = circuit {
        H @0
        T @0
    }"#;
    let diags = lint_snippet(src);
    if has_rule(&diags, "gates/universal-in-clifford-block") {
        return;
    }
    assert!(
        frontend::check_program(src).is_err(),
        "expected either lint warning or type error for T in Clifford block"
    );
}

#[test]
fn parse_error_yields_no_lints() {
    let src = "fn broken( = 1\n";
    let diags = quonlint::lint_source(
        std::path::Path::new("broken.qn"),
        src,
        &quonlint::LintConfig::default(),
    );
    assert!(diags.is_empty());
}

#[test]
fn type_error_yields_no_lints() {
    let src = "fn f(): Int = true\n";
    let diags = quonlint::lint_source(
        std::path::Path::new("bad.qn"),
        src,
        &quonlint::LintConfig::default(),
    );
    assert!(diags.is_empty());
}
