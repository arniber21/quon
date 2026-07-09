//! Lower [`CircuitSpec`] to a `quantum.circ` module and shared test helpers.

use melior::Context;
use melior::ir::{Block, BlockLike, Location, Module, OperationRef, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use proptest::test_runner::Config as ProptestConfig;
use quon_core::DepthExpr;

use super::circuit_spec::{CircuitSpec, GateInst, GateKind};

/// Melior context with `quantum.circ` registered — create **inside** each trial.
pub fn context() -> Context {
    let context = Context::new();
    qc::register_dialect(&context);
    context
}

/// Default case budget for pass property suites (~64–128; CI-friendly).
pub fn prop_config() -> ProptestConfig {
    ProptestConfig {
        cases: 96,
        ..ProptestConfig::default()
    }
}

fn build_body<'c>(
    context: &'c Context,
    spec: &CircuitSpec,
    location: Location<'c>,
) -> (Region<'c>, bool) {
    let qubit = qc::qubit_type(context);
    let width = spec.width as usize;
    let block = Block::new(&vec![(qubit, location); width]);
    let mut wires: Vec<Value<'c, '_>> = (0..width)
        .map(|i| Value::from(block.argument(i).expect("arg")))
        .collect();
    for gate in &spec.gates {
        let operands: Vec<Value<'c, '_>> = gate.qubits.iter().map(|&q| wires[q as usize]).collect();
        let op = if gate.kind.is_rotation() {
            let angle = gate.angle.unwrap_or(0.0);
            block.append_operation(
                qc::rotation_gate(
                    context,
                    gate.kind.mlir_name(),
                    angle,
                    1,
                    false,
                    operands[0],
                    location,
                )
                .expect("rotation gate"),
            )
        } else {
            let clifford = gate.kind.is_clifford();
            block.append_operation(
                qc::gate(
                    context,
                    gate.kind.mlir_name(),
                    1,
                    clifford,
                    &operands,
                    location,
                )
                .expect("gate"),
            )
        };
        for (index, &q) in gate.qubits.iter().enumerate() {
            wires[q as usize] = Value::from(op.result(index).expect("result"));
        }
    }
    block.append_operation(qc::r#return(&wires, location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    let clifford = spec.all_clifford();
    (region, clifford)
}

/// Lowers `spec` to a module containing a single `quantum.circ.func @main`.
pub fn lower_func_module<'c>(context: &'c Context, spec: &CircuitSpec) -> Module<'c> {
    let location = Location::unknown(context);
    let (region, clifford) = build_body(context, spec, location);
    let width = i64::from(spec.width);
    let func = qc::func(
        context,
        "main",
        width,
        width,
        &DepthExpr::Nat(spec.depth() as u64),
        clifford,
        region,
        location,
    )
    .expect("func");
    let module = Module::new(location);
    module.body().append_operation(func);
    module
}

/// Lowers `spec` to a top-level `quantum.circ.borrow` (uncomputation target).
pub fn lower_borrow_module<'c>(context: &'c Context, spec: &CircuitSpec) -> Module<'c> {
    let location = Location::unknown(context);
    let (region, _) = build_body(context, spec, location);
    let borrow = qc::borrow(
        context,
        spec.width as usize,
        &DepthExpr::Nat(spec.depth() as u64),
        region,
        location,
    )
    .expect("borrow");
    let module = Module::new(location);
    module.body().append_operation(borrow);
    module
}

/// First top-level op in the module body (func or borrow).
pub fn read_func_op<'c, 'a>(module: &'a Module<'c>) -> OperationRef<'c, 'a> {
    module.body().first_operation().expect("top-level op")
}

/// Seeded adversarial circuits that historically stress cancellation / merge.
pub fn seed_adversarial() -> Vec<CircuitSpec> {
    use GateKind::*;
    vec![
        CircuitSpec::new(
            1,
            vec![GateInst::new(H, vec![0]), GateInst::new(H, vec![0])],
        ),
        CircuitSpec::new(
            2,
            vec![
                GateInst::new(CNOT, vec![0, 1]),
                GateInst::new(CNOT, vec![0, 1]),
            ],
        ),
        CircuitSpec::new(
            1,
            vec![
                GateInst::rotation(Rz, 0, 0.5),
                GateInst::rotation(Rz, 0, 0.3),
            ],
        ),
        CircuitSpec::new(
            1,
            vec![GateInst::new(T, vec![0]), GateInst::new(Tdag, vec![0])],
        ),
        CircuitSpec::new(
            1,
            vec![GateInst::new(X, vec![0]), GateInst::new(X, vec![0])],
        ),
        CircuitSpec::new(
            3,
            vec![
                GateInst::new(H, vec![0]),
                GateInst::new(H, vec![1]),
                GateInst::new(H, vec![2]),
                GateInst::new(CNOT, vec![0, 2]),
            ],
        ),
        CircuitSpec::new(
            1,
            vec![
                GateInst::rotation(Rz, 0, std::f64::consts::PI),
                GateInst::rotation(Rz, 0, std::f64::consts::PI),
            ],
        ),
        CircuitSpec::new(
            2,
            vec![
                GateInst::new(CNOT, vec![0, 1]),
                GateInst::new(CNOT, vec![1, 0]),
            ],
        ),
    ]
}
