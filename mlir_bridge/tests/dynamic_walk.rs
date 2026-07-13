//! Shared `dynamic_walk` edge-case tests (issue #208).

mod support;

use melior::ir::attribute::StringAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::{Block, BlockLike, Location, Module, OperationRef, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use mlir_bridge::dynamic_walk::{self, DynamicVisitor, IfArm};
use mlir_bridge::metrics::collect_module_metrics;
use quon_core::DepthExpr;

fn append_gate<'c: 'a, 'a, B: BlockLike<'c, 'a>>(
    context: &'c melior::Context,
    block: &B,
    name: &str,
    q: Value<'c, 'a>,
    location: Location<'c>,
) -> Value<'c, 'a> {
    let op = block.append_operation(qc::gate(context, name, 1, true, &[q], location).unwrap());
    Value::from(op.result(0).unwrap())
}

fn gate_name<'c: 'a, 'a>(op: OperationRef<'c, 'a>) -> String {
    op.attribute(qc::attr::GATE_NAME)
        .ok()
        .and_then(|value| StringAttribute::try_from(value).ok())
        .map(|string| string.value().to_string())
        .unwrap_or_default()
}

#[derive(Default)]
struct EventRecorder {
    events: Vec<String>,
}

impl<'c, 'a> DynamicVisitor<'c, 'a> for EventRecorder {
    fn gate(&mut self, op: OperationRef<'c, 'a>, _qubit_roots: &[usize]) {
        self.events.push(format!("gate:{}", gate_name(op)));
    }

    fn enter_unitary_region(&mut self, _op: OperationRef<'c, 'a>, _qubit_roots: &[usize]) {
        self.events.push("enter_unitary".into());
    }

    fn exit_unitary_region(&mut self, _op: OperationRef<'c, 'a>) {
        self.events.push("exit_unitary".into());
    }

    fn enter_if_arm(&mut self, _op: OperationRef<'c, 'a>, arm: IfArm, _qubit_roots: &[usize]) {
        self.events.push(format!(
            "enter_if:{}",
            match arm {
                IfArm::Then => "then",
                IfArm::Else => "else",
            }
        ));
    }

    fn exit_if_arm(&mut self, _op: OperationRef<'c, 'a>, arm: IfArm) {
        self.events.push(format!(
            "exit_if:{}",
            match arm {
                IfArm::Then => "then",
                IfArm::Else => "else",
            }
        ));
    }
}

#[test]
fn walks_into_unitary_region_and_nested_if_arm() {
    let context = support::dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);

    let module = Module::new(location);
    let body = module.body();
    let q_meas = support::append_foreign_qubit(&context, &body, location);
    let q0 = support::append_foreign_qubit(&context, &body, location);

    let measure = body.append_operation(qd::measure(&context, q_meas, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    // then: unitary_region { H }
    let then_region = Region::new();
    let then_block = Block::new(&[(qubit, location)]);
    let then_arg = Value::from(then_block.argument(0).unwrap());

    let unitary_body = Region::new();
    let unitary_block = Block::new(&[(qubit, location)]);
    let u_arg = Value::from(unitary_block.argument(0).unwrap());
    let u_out = append_gate(&context, &unitary_block, "H", u_arg, location);
    unitary_block.append_operation(qc::r#return(&[u_out], location).unwrap());
    unitary_body.append_block(unitary_block);

    let ur = then_block.append_operation(
        qd::unitary_region(
            &context,
            &[then_arg],
            &DepthExpr::Nat(1),
            true,
            unitary_body,
            location,
        )
        .expect("unitary_region"),
    );
    let then_out = Value::from(ur.result(0).unwrap());
    then_block.append_operation(qd::r#yield(&[then_out], location).unwrap());
    then_region.append_block(then_block);

    // else: X
    let else_region = Region::new();
    let else_block = Block::new(&[(qubit, location)]);
    let else_arg = Value::from(else_block.argument(0).unwrap());
    let else_out = append_gate(&context, &else_block, "X", else_arg, location);
    else_block.append_operation(qd::r#yield(&[else_out], location).unwrap());
    else_region.append_block(else_block);

    body.append_operation(
        qd::r#if(&context, bit, &[q0], then_region, else_region, location).expect("if"),
    );

    let mut recorder = EventRecorder::default();
    dynamic_walk::walk_block(body, &mut recorder);
    assert_eq!(
        recorder.events,
        vec![
            "enter_if:then",
            "enter_unitary",
            "gate:H",
            "exit_unitary",
            "exit_if:then",
            "enter_if:else",
            "gate:X",
            "exit_if:else",
        ]
    );

    let target = backend::BackendTarget::fixed(
        "test",
        backend::FixedTarget {
            num_qubits: 2,
            topology: backend::ConnectivityGraph::all_to_all(2),
            native_gates: vec![
                backend::NativeGate::passthrough("H", 1),
                backend::NativeGate::passthrough("X", 1),
            ],
            noise: backend::NoiseModel::default(),
            meas_latency_us: 0.0,
            supports_mid_circuit_meas: true,
            supports_feed_forward: true,
        },
    );
    let metrics = collect_module_metrics(&module, &target);
    assert_eq!(
        metrics.gate_count, 2,
        "gates inside nested unitary_region and else arm must be counted"
    );
}

#[test]
fn both_if_arms_are_visited() {
    let context = support::dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);

    let module = Module::new(location);
    let body = module.body();
    let q_meas = support::append_foreign_qubit(&context, &body, location);
    let q_branch = support::append_foreign_qubit(&context, &body, location);

    let measure = body.append_operation(qd::measure(&context, q_meas, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    let then_region = Region::new();
    let then_block = Block::new(&[(qubit, location)]);
    let then_arg = Value::from(then_block.argument(0).unwrap());
    let then_out = append_gate(&context, &then_block, "H", then_arg, location);
    then_block.append_operation(qd::r#yield(&[then_out], location).unwrap());
    then_region.append_block(then_block);

    let else_region = Region::new();
    let else_block = Block::new(&[(qubit, location)]);
    let else_arg = Value::from(else_block.argument(0).unwrap());
    let else_out = append_gate(&context, &else_block, "T", else_arg, location);
    else_block.append_operation(qd::r#yield(&[else_out], location).unwrap());
    else_region.append_block(else_block);

    body.append_operation(
        qd::r#if(
            &context,
            bit,
            &[q_branch],
            then_region,
            else_region,
            location,
        )
        .expect("if"),
    );

    let mut recorder = EventRecorder::default();
    dynamic_walk::walk_block(body, &mut recorder);
    assert_eq!(
        recorder.events,
        vec![
            "enter_if:then",
            "gate:H",
            "exit_if:then",
            "enter_if:else",
            "gate:T",
            "exit_if:else",
        ]
    );

    let target = backend::BackendTarget::fixed(
        "test",
        backend::FixedTarget {
            num_qubits: 2,
            topology: backend::ConnectivityGraph::all_to_all(2),
            native_gates: vec![
                backend::NativeGate::passthrough("H", 1),
                backend::NativeGate::passthrough("T", 1),
            ],
            noise: backend::NoiseModel::default(),
            meas_latency_us: 0.0,
            supports_mid_circuit_meas: true,
            supports_feed_forward: true,
        },
    );
    let metrics = collect_module_metrics(&module, &target);
    assert_eq!(metrics.gate_count, 2);
    assert_eq!(metrics.t_count, 1);
}
