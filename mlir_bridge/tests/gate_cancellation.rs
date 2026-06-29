//! Gate cancellation integration tests (issue #18).

mod support;

use melior::ir::attribute::StringAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::gate_cancellation;
use quon_core::DepthExpr;

use support::context;

fn build_func<'c>(
    context: &'c melior::Context,
    gates: &[(&str, &[usize])],
    depth: &DepthExpr,
) -> melior::ir::Operation<'c> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let in_qubits = gates
        .iter()
        .flat_map(|(_, targets)| targets.iter().copied())
        .max()
        .map(|index| index + 1)
        .unwrap_or(1)
        .max(1) as i64;
    let block = Block::new(&vec![(qubit, location); in_qubits as usize]);
    let mut wires: Vec<Value<'c, '_>> = (0..block.argument_count())
        .map(|index| Value::from(block.argument(index).expect("arg")))
        .collect();
    for (gate_name, targets) in gates {
        let operands: Vec<Value<'c, '_>> = targets.iter().map(|index| wires[*index]).collect();
        let op = block.append_operation(
            qc::gate(context, gate_name, 1, true, &operands, location).expect("gate"),
        );
        for (index, target) in targets.iter().enumerate() {
            wires[*target] = Value::from(op.result(index).expect("result"));
        }
    }
    block.append_operation(qc::r#return(&wires, location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    qc::func(
        context, "main", in_qubits, in_qubits, depth, true, region, location,
    )
    .expect("func")
}

fn count_gate(module: &Module<'_>, gate_name: &str) -> usize {
    let text = module.as_operation().to_string();
    text.matches(&format!("gate_name = \"{gate_name}\""))
        .count()
}

fn read_func_depth(module: &Module<'_>) -> DepthExpr {
    let func = module.body().first_operation().expect("func");
    let attr = func.attribute(qc::attr::DEPTH).expect("depth attr");
    let text = StringAttribute::try_from(attr)
        .expect("string depth")
        .value()
        .to_string();
    DepthExpr::parse(&text).expect("depth expr")
}

#[test]
fn h_h_cancels_to_identity() {
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("H", &[0]), ("H", &[0])],
        &DepthExpr::Nat(2),
    ));
    gate_cancellation::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "H"), 0);
    assert_eq!(read_func_depth(&module), DepthExpr::Nat(0));
}

#[test]
fn cnot_cnot_cancels_when_qubits_match() {
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("CNOT", &[0, 1]), ("CNOT", &[0, 1])],
        &DepthExpr::Nat(2),
    ));
    gate_cancellation::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "CNOT"), 0);
}

#[test]
fn h_t_h_is_not_cancelled() {
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("H", &[0]), ("T", &[0]), ("H", &[0])],
        &DepthExpr::Nat(3),
    ));
    gate_cancellation::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "H"), 2);
    assert_eq!(count_gate(&module, "T"), 1);
}

#[test]
fn mismatched_cnot_controls_are_not_cancelled() {
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("CNOT", &[0, 1]), ("CNOT", &[1, 0])],
        &DepthExpr::Nat(2),
    ));
    gate_cancellation::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "CNOT"), 2);
}
