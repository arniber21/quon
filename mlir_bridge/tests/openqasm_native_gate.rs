//! Locks the `native_gate = false` branch in `emit/openqasm3.rs` (issue #499).
//!
//! An explicit `false` fails reify and emit before `resolve_gate`, including
//! for identity (which would otherwise be elided) and for a native keyword
//! such as `cx`. A missing attribute still goes through `resolve_gate`, so
//! identity elides and `x` is emitted.

mod support;

use backend::generic_openqasm;
use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Type, Value};
use quon_core::DepthExpr;
use quon_core::qasm::{QasmGate, Stmt};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use mlir_bridge::emit::openqasm3::{self, EmitError};

use support::{bool_attr, dynamic_context, generic_op, i64_attr, str_attr};

fn flagged_gate_module<'c>(
    context: &'c melior::Context,
    gate_name: &str,
    arity: usize,
) -> Module<'c> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let module = Module::new(location);
    let body = module.body();
    let qubits: Vec<Value> = (0..arity)
        .map(|_| support::append_foreign_qubit(context, &body, location))
        .collect();

    let args: Vec<(Type, Location)> = (0..arity).map(|_| (qubit, location)).collect();
    let inner = Block::new(&args);
    let operands: Vec<Value> = (0..arity)
        .map(|index| Value::from(inner.argument(index).expect("region argument")))
        .collect();
    let results = vec![qubit; arity];
    let gate = inner.append_operation(generic_op(
        context,
        qc::op::GATE,
        &operands,
        &results,
        &[
            (qc::attr::GATE_NAME, str_attr(context, gate_name)),
            (qc::attr::DEPTH_CONTRIBUTION, i64_attr(context, 1)),
            (qc::attr::CLIFFORD, bool_attr(context, true)),
            (qd::attr::NATIVE_GATE, bool_attr(context, false)),
        ],
        vec![],
        location,
    ));
    let outputs: Vec<Value> = (0..arity)
        .map(|index| Value::from(gate.result(index).expect("gate result")))
        .collect();
    inner.append_operation(qc::r#return(&outputs, location).expect("return"));
    let region = Region::new();
    region.append_block(inner);
    body.append_operation(
        qd::unitary_region(context, &qubits, &DepthExpr::Nat(1), true, region, location)
            .expect("unitary_region"),
    );
    module
}

fn assert_false_flag_rejected(gate_name: &str, arity: usize) {
    let context = dynamic_context();
    let target = generic_openqasm::target(arity);
    let module = flagged_gate_module(&context, gate_name, arity);
    match openqasm3::reify(&module, &target) {
        Err(EmitError::NonNativeGate { name, target }) => {
            assert_eq!(name, gate_name);
            assert_eq!(target, "generic_openqasm");
        }
        other => panic!("reify of {gate_name} with native_gate=false: {other:?}"),
    }
    match openqasm3::emit(&module, &target) {
        Err(EmitError::NonNativeGate { name, target }) => {
            assert_eq!(name, gate_name);
            assert_eq!(target, "generic_openqasm");
        }
        other => panic!("emit of {gate_name} with native_gate=false: {other:?}"),
    }
}

/// `I` then `x`, neither carrying `native_gate`. Identity must elide.
fn identity_then_x<'c>(context: &'c melior::Context) -> Module<'c> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let module = Module::new(location);
    let body = module.body();
    let q = support::append_foreign_qubit(context, &body, location);

    let inner = Block::new(&[(qubit, location)]);
    let arg = Value::from(inner.argument(0).expect("region argument"));
    let ident = inner.append_operation(
        qc::gate(context, "I", 1, true, &[arg], location).expect("identity gate"),
    );
    let after_i = Value::from(ident.result(0).expect("identity result"));
    let x = inner
        .append_operation(qc::gate(context, "x", 1, true, &[after_i], location).expect("x gate"));
    let after_x = Value::from(x.result(0).expect("x result"));
    inner.append_operation(qc::r#return(&[after_x], location).expect("return"));
    let region = Region::new();
    region.append_block(inner);
    body.append_operation(
        qd::unitary_region(context, &[q], &DepthExpr::Nat(1), true, region, location)
            .expect("unitary_region"),
    );
    module
}

#[test]
fn native_gate_false_fails_reify_and_emit() {
    // A native OpenQASM keyword still flagged false is not native yet.
    assert_false_flag_rejected("cx", 2);
    // Identity would elide in `resolve_gate`; the flag must fail first.
    assert_false_flag_rejected("I", 1);
}

#[test]
fn missing_native_gate_attribute_elides_identity() {
    let context = dynamic_context();
    let target = generic_openqasm::target(1);
    let module = identity_then_x(&context);
    let program = openqasm3::reify(&module, &target).expect("reify");
    match program.body() {
        [Stmt::Gate(QasmGate::Std1 { keyword: "x", .. })] => {}
        other => panic!("identity should elide and leave only x, got {other:?}"),
    }
    let text = openqasm3::emit(&module, &target).expect("emit");
    assert!(
        text.contains("x q[0];"),
        "missing attribute should still resolve x, got {text}"
    );
    assert!(
        !text.contains("id ") && !text.contains(" I "),
        "identity must not be printed, got {text}"
    );
}
