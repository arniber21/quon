//! Clifford+T optimization integration tests (issue #96).
//!
//! Tests the `clifford_t_opt` pass on MLIR modules:
//! - Stabilizer tableau identity detection (clifford=true funcs)
//! - Phase polynomial T-count reduction (clifford=false funcs)
//! - Non-adjacent gate handling

mod support;

use melior::ir::attribute::StringAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::clifford_t_opt;
use quon_core::DepthExpr;

use support::context;

/// Build a `quantum.circ.func` with a flat gate sequence.
fn build_func<'c>(
    context: &'c melior::Context,
    gates: &[(&str, &[usize])],
    depth: &DepthExpr,
    clifford: bool,
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
        let is_clifford = quon_core::gates::lookup(gate_name)
            .is_some_and(|g| g.class == quon_core::gates::GateClass::Clifford);
        let op = block.append_operation(
            qc::gate(context, gate_name, 1, is_clifford, &operands, location).expect("gate"),
        );
        for (index, target) in targets.iter().enumerate() {
            wires[*target] = Value::from(op.result(index).expect("result"));
        }
    }
    block.append_operation(qc::r#return(&wires, location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    qc::func(
        context, "main", in_qubits, in_qubits, depth, clifford, region, location,
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

// ===========================================================================
// Stabilizer tableau: identity detection (clifford=true)
// ===========================================================================

#[test]
fn s_four_times_is_identity() {
    // S · S · S · S = I — not caught by gate_cancellation (S is not self-inverse)
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("S", &[0]), ("S", &[0]), ("S", &[0]), ("S", &[0])],
        &DepthExpr::Nat(4),
        true,
    ));
    clifford_t_opt::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "S"), 0);
    assert_eq!(read_func_depth(&module), DepthExpr::Nat(0));
}

#[test]
fn h_s4_h_is_identity() {
    // H · S⁴ · H = H · I · H = I — non-adjacent (S gates between H gates)
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[
            ("H", &[0]),
            ("S", &[0]),
            ("S", &[0]),
            ("S", &[0]),
            ("S", &[0]),
            ("H", &[0]),
        ],
        &DepthExpr::Nat(6),
        true,
    ));
    clifford_t_opt::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "H"), 0);
    assert_eq!(count_gate(&module, "S"), 0);
    assert_eq!(read_func_depth(&module), DepthExpr::Nat(0));
}

#[test]
fn s_squared_collapses_to_z() {
    // S · S = Z — single Pauli detection
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("S", &[0]), ("S", &[0])],
        &DepthExpr::Nat(2),
        true,
    ));
    clifford_t_opt::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "S"), 0);
    assert_eq!(count_gate(&module, "Z"), 1);
}

#[test]
fn hadamard_is_not_identity() {
    // H alone is not identity — should be preserved
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("H", &[0])],
        &DepthExpr::Nat(1),
        true,
    ));
    clifford_t_opt::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "H"), 1);
}

#[test]
fn multi_qubit_swap_swap_is_identity() {
    // SWAP · SWAP = I — 6 CNOTs, non-adjacent
    let context = context();
    let module = Module::new(Location::unknown(&context));
    let swap: [(&str, &[usize]); 3] = [("CNOT", &[0, 1]), ("CNOT", &[1, 0]), ("CNOT", &[0, 1])];
    let gates: Vec<(&str, &[usize])> = swap.iter().chain(swap.iter()).copied().collect();
    module
        .body()
        .append_operation(build_func(&context, &gates, &DepthExpr::Nat(6), true));
    clifford_t_opt::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "CNOT"), 0);
}

// ===========================================================================
// Phase polynomial: T-count reduction (clifford=false)
// ===========================================================================

#[test]
fn non_adjacent_t_pair_merges_to_s() {
    // T(0), CNOT(0,1), T(0) → both T on parity {0}, merge to S — T-count 2→0
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("T", &[0]), ("CNOT", &[0, 1]), ("T", &[0])],
        &DepthExpr::Nat(3),
        false,
    ));
    clifford_t_opt::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "T"), 0);
    // S should appear (the merged T² = S)
    assert_eq!(count_gate(&module, "S"), 1);
    assert!(count_gate(&module, "CNOT") >= 1);
}

#[test]
fn three_t_on_same_parity_reduces_to_one() {
    // T(0), CNOT(0,1), T(0), CNOT(0,1), T(0) → T³ on parity {0} — T-count 3→1
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[
            ("T", &[0]),
            ("CNOT", &[0, 1]),
            ("T", &[0]),
            ("CNOT", &[0, 1]),
            ("T", &[0]),
        ],
        &DepthExpr::Nat(5),
        false,
    ));
    clifford_t_opt::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "T"), 1);
}

#[test]
fn t_dag_cancels_t_non_adjacent() {
    // T(0), CNOT(0,1), T_dag(0) → T·T† = I on parity {0} — T-count 2→0
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("T", &[0]), ("CNOT", &[0, 1]), ("T_dag", &[0])],
        &DepthExpr::Nat(3),
        false,
    ));
    clifford_t_opt::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "T"), 0);
    assert_eq!(count_gate(&module, "T_dag"), 0);
}

#[test]
fn t_h_t_is_not_reduced() {
    // T(0), H(0), T(0) — H splits the CNOT+T blocks, no merging possible
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("T", &[0]), ("H", &[0]), ("T", &[0])],
        &DepthExpr::Nat(3),
        false,
    ));
    clifford_t_opt::run_on_module(&context, &module);
    // T-count should remain 2 — H prevents merging
    assert_eq!(count_gate(&module, "T"), 2);
}

#[test]
fn different_parity_t_not_merged() {
    // T(1), CNOT(0,1), T(1) — second T has parity {0,1} ≠ {1} — no merge
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("T", &[1]), ("CNOT", &[0, 1]), ("T", &[1])],
        &DepthExpr::Nat(3),
        false,
    ));
    clifford_t_opt::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "T"), 2);
}

#[test]
fn empty_circuit_unchanged() {
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module
        .body()
        .append_operation(build_func(&context, &[], &DepthExpr::Nat(0), false));
    clifford_t_opt::run_on_module(&context, &module);
    // Should not crash, no gates to optimize
}

#[test]
fn non_gate_ops_skipped() {
    // A func with only H (non-T, non-CNOT) — should be preserved for clifford=false
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_func(
        &context,
        &[("H", &[0])],
        &DepthExpr::Nat(1),
        false,
    ));
    clifford_t_opt::run_on_module(&context, &module);
    assert_eq!(count_gate(&module, "H"), 1);
}
