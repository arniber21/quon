//! AST → `quantum.circ` lowering tests (issue #16).

use melior::Context;
use melior::ir::BlockLike;
use melior::ir::operation::OperationLike;
use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::linearity_verifier::check_linearity;

use frontend::lower::lower_program;

fn lower_text(src: &str) -> String {
    let context = Context::new();
    let module = lower_program(&context, src).expect("lowering should succeed");
    module.as_operation().to_string()
}

fn module_linearity_ok(module: &melior::ir::Module<'_>) -> bool {
    let body = module.body();
    let mut op = body.first_operation();
    while let Some(current) = op {
        let op_name = current
            .name()
            .as_string_ref()
            .as_str()
            .unwrap_or("")
            .to_string();
        if op_name == qc::op::FUNC && !check_linearity(&current).is_empty() {
            return false;
        }
        op = current.next_in_block();
    }
    true
}

#[test]
fn bell_state_lowers_to_quantum_circ_func() {
    let src = include_str!("fixtures/bell_state.qn");
    let text = lower_text(src);
    assert!(
        text.contains(r#"sym_name = "bell_state""#),
        "missing bell_state func: {text}"
    );
    assert!(text.contains("in_qubits = 2"));
    assert!(text.contains("out_qubits = 2"));
    assert!(text.contains(r#"depth = "2""#));
    assert!(text.contains("clifford = true"));
    assert!(text.contains(r#"gate_name = "H""#));
    assert!(text.contains(r#"gate_name = "CNOT""#));
}

#[test]
fn gate_ops_carry_depth_and_clifford_attributes() {
    let src = "fn f(): Circuit<1, 1, 1, Clifford> = circuit { H @0 }";
    let text = lower_text(src);
    assert!(text.contains("depth_contribution = 1"));
    assert!(text.contains("clifford = true"));
    assert!(text.contains(r#"gate_name = "H""#));
}

#[test]
fn adjoint_of_circuit_call_emits_adjoint_op() {
    let src = "\
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit { H @0 |> CNOT @(0, 1) }
fn adjoint_bell(): Circuit<2, 2, 2, Clifford> = adjoint(bell_state())
";
    let text = lower_text(src);
    assert!(text.contains("quantum.circ.adjoint"));
    assert!(text.contains(r#"sym_name = "bell_state""#));
}

#[test]
fn lowered_bell_state_passes_linearity_verifier() {
    let src = "fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit { H @0 |> CNOT @(0, 1) }";
    let context = Context::new();
    let module = lower_program(&context, src).expect("lower bell_state");
    assert!(module_linearity_ok(&module), "bell_state should be linear");
}

#[test]
fn bell_state_fixture_passes_linearity_verifier() {
    let src = include_str!("fixtures/bell_state.qn");
    let context = Context::new();
    let module = lower_program(&context, src).expect("lower bell_state fixture");
    assert!(
        module_linearity_ok(&module),
        "bell_state fixture should be linear"
    );
}
