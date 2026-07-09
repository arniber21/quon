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
    assert!(rules.len() >= 12);
}
