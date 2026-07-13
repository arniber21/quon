//! Interaction-graph extraction from `quantum.dynamic` / `quantum.circ` fixtures.

use melior::Context;
use melior::ir::operation::{OperationBuilder, OperationLike};
use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use quon_core::DepthExpr;
use quon_na::{DEFAULT_GAMMA, LogicalQubitId, SegmentKind, extract_interaction_graph};

fn dynamic_context() -> Context {
    let context = Context::new();
    qc::register_dialect(&context);
    qd::register_dialect(&context);
    context
}

fn append_gate<'c: 'a, 'a, B: BlockLike<'c, 'a>>(
    context: &'c Context,
    block: &B,
    name: &str,
    qubits: &[Value<'c, 'a>],
    location: Location<'c>,
) -> Vec<Value<'c, 'a>> {
    let op =
        block.append_operation(qc::gate(context, name, 1, true, qubits, location).expect("gate"));
    op.results().map(Value::from).collect()
}

fn foreign_qubit<'c: 'a, 'a, B: BlockLike<'c, 'a>>(
    context: &'c Context,
    body: &B,
    location: Location<'c>,
) -> Value<'c, 'a> {
    Value::from(
        body.append_operation(
            OperationBuilder::new("test.qubit", location)
                .add_results(&[qc::qubit_type(context)])
                .build()
                .expect("foreign qubit"),
        )
        .result(0)
        .expect("result"),
    )
}

#[test]
fn bell_extracts_one_cx_edge() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let out = append_gate(&context, &block, "CX", &[q0, q1], location);
    block.append_operation(qc::r#return(&out, location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "bell",
        2,
        2,
        &DepthExpr::Nat(1),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    module.body().append_operation(func);

    let graph = extract_interaction_graph(&module).unwrap();
    assert_eq!(graph.vertices, vec![LogicalQubitId(0), LogicalQubitId(1)]);
    assert_eq!(graph.interactions.len(), 1);
    assert_eq!(graph.edges.len(), 1);
    assert!((graph.edges[0].weight - 1.0).abs() < 1e-12);
    assert!(graph.interactions[0].on_critical_path);
    assert_eq!(graph.interactions[0].dag_layer, 0);
    assert_eq!(graph.segments.len(), 1);
    assert_eq!(graph.segments[0].kind, SegmentKind::DependencyDag);
}

#[test]
fn sequential_shared_qubit_gets_layers() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location), (qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let q2 = Value::from(block.argument(2).unwrap());
    let mid = append_gate(&context, &block, "CX", &[q0, q1], location);
    let out = append_gate(&context, &block, "CX", &[mid[1], q2], location);
    block.append_operation(qc::r#return(&[mid[0], out[0], out[1]], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "chain",
        3,
        3,
        &DepthExpr::Nat(2),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    module.body().append_operation(func);

    let graph = extract_interaction_graph(&module).unwrap();
    assert_eq!(graph.interactions.len(), 2);
    assert_eq!(graph.interactions[0].dag_layer, 0);
    assert_eq!(graph.interactions[1].dag_layer, 1);
    assert!(graph.interactions.iter().all(|i| i.on_critical_path));
    assert_eq!(graph.edges.len(), 2);
    let w01 = graph
        .edges
        .iter()
        .find(|e| e.a == LogicalQubitId(0) && e.b == LogicalQubitId(1))
        .unwrap();
    let w12 = graph
        .edges
        .iter()
        .find(|e| e.a == LogicalQubitId(1) && e.b == LogicalQubitId(2))
        .unwrap();
    assert!((w01.weight - 1.0).abs() < 1e-12);
    assert!((w12.weight - DEFAULT_GAMMA).abs() < 1e-12);
}

#[test]
fn barrier_splits_dependency_segments() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let mid = append_gate(&context, &block, "CX", &[q0, q1], location);
    let barred = block
        .append_operation(qd::barrier(&context, &mid, location).unwrap())
        .results()
        .map(Value::from)
        .collect::<Vec<_>>();
    let out = append_gate(&context, &block, "CX", &[barred[0], barred[1]], location);
    block.append_operation(qc::r#return(&out, location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "barred",
        2,
        2,
        &DepthExpr::Nat(2),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    module.body().append_operation(func);

    let graph = extract_interaction_graph(&module).unwrap();
    assert_eq!(graph.interactions.len(), 2);
    assert_eq!(graph.segments.len(), 2);
    assert!(
        graph
            .segments
            .iter()
            .all(|s| s.kind == SegmentKind::DependencyDag)
    );
    assert_eq!(graph.edges.len(), 1);
    assert!((graph.edges[0].weight - 2.0).abs() < 1e-12);
}

#[test]
fn if_counts_both_arms() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let module = Module::new(location);
    let body = module.body();

    let q0 = foreign_qubit(&context, &body, location);
    let q1 = foreign_qubit(&context, &body, location);
    let q2 = foreign_qubit(&context, &body, location);

    let measure = body.append_operation(qd::measure(&context, q0, location).unwrap());
    let bit = Value::from(measure.result(0).unwrap());

    let then_region = Region::new();
    let then_block = Block::new(&[(qubit, location), (qubit, location)]);
    let t0 = Value::from(then_block.argument(0).unwrap());
    let t1 = Value::from(then_block.argument(1).unwrap());
    let tout = append_gate(&context, &then_block, "CX", &[t0, t1], location);
    then_block.append_operation(qd::r#yield(&tout, location).unwrap());
    then_region.append_block(then_block);

    let else_region = Region::new();
    let else_block = Block::new(&[(qubit, location), (qubit, location)]);
    let e0 = Value::from(else_block.argument(0).unwrap());
    let e1 = Value::from(else_block.argument(1).unwrap());
    let eout = append_gate(&context, &else_block, "CX", &[e0, e1], location);
    else_block.append_operation(qd::r#yield(&eout, location).unwrap());
    else_region.append_block(else_block);

    body.append_operation(
        qd::r#if(&context, bit, &[q1, q2], then_region, else_region, location).unwrap(),
    );

    let graph = extract_interaction_graph(&module).unwrap();
    assert_eq!(graph.interactions.len(), 2, "both if arms contribute");
}

#[test]
fn isolated_qubit_appears_in_vertices() {
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location), (qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let q2 = Value::from(block.argument(2).unwrap());
    let out = append_gate(&context, &block, "CX", &[q0, q1], location);
    block.append_operation(qc::r#return(&[out[0], out[1], q2], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "isolated",
        3,
        3,
        &DepthExpr::Nat(1),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    module.body().append_operation(func);

    let graph = extract_interaction_graph(&module).unwrap();
    assert_eq!(
        graph.vertices,
        vec![LogicalQubitId(0), LogicalQubitId(1), LogicalQubitId(2)]
    );
    assert_eq!(graph.edges.len(), 1);
    let mut degree = [0u32; 3];
    for e in &graph.edges {
        degree[e.a.0 as usize] += 1;
        degree[e.b.0 as usize] += 1;
    }
    assert_eq!(degree[2], 0);
}

#[test]
fn top_level_program_ignores_leftover_circ_func() {
    // Simulates post-monadic-lowering: executed program on the module body
    // plus a leftover inlined `quantum.circ.func` callee. Extract must not
    // merge both (pre-#112 bug: Bell reported 4 logical qubits / 2 CXs).
    let context = dynamic_context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let module = Module::new(location);
    let body = module.body();

    // Top-level executed CX.
    let q0 = foreign_qubit(&context, &body, location);
    let q1 = foreign_qubit(&context, &body, location);
    let _ = append_gate(&context, &body, "CX", &[q0, q1], location);

    // Dead leftover callee with its own CX.
    let func_block = Block::new(&[(qubit, location), (qubit, location)]);
    let fq0 = Value::from(func_block.argument(0).unwrap());
    let fq1 = Value::from(func_block.argument(1).unwrap());
    let fout = append_gate(&context, &func_block, "CX", &[fq0, fq1], location);
    func_block.append_operation(qc::r#return(&fout, location).unwrap());
    let region = Region::new();
    region.append_block(func_block);
    body.append_operation(
        qc::func(
            &context,
            "dead_callee",
            2,
            2,
            &DepthExpr::Nat(1),
            true,
            region,
            location,
        )
        .unwrap(),
    );

    let graph = extract_interaction_graph(&module).unwrap();
    assert_eq!(
        graph.vertices.len(),
        2,
        "must not merge leftover circ.func qubits"
    );
    assert_eq!(
        graph.interactions.len(),
        1,
        "must not double-count the callee CX"
    );
    assert_eq!(graph.vertices, vec![LogicalQubitId(0), LogicalQubitId(1)]);
}
