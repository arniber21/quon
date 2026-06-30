//! Rotation merging integration tests (issue #19).

mod support;

use std::f64::consts::PI;

use melior::ir::attribute::StringAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::rotation_merging;
use quon_core::DepthExpr;

use support::context;

fn build_rotation_func<'c>(
    context: &'c melior::Context,
    gates: &[(&str, f64)],
    depth: &DepthExpr,
) -> melior::ir::Operation<'c> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location)]);
    let mut wire = Value::from(block.argument(0).expect("arg"));
    for (axis, angle) in gates {
        let op = block.append_operation(
            qc::rotation_gate(context, axis, *angle, 1, false, wire, location).expect("rotation"),
        );
        wire = Value::from(op.result(0).expect("result"));
    }
    block.append_operation(qc::r#return(&[wire], location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    qc::func(context, "main", 1, 1, depth, false, region, location).expect("func")
}

fn count_rotation(module: &Module<'_>, axis: &str) -> usize {
    module
        .as_operation()
        .to_string()
        .matches(&format!("gate_name = \"{axis}\""))
        .count()
}

fn read_angle(module: &Module<'_>) -> f64 {
    let text = module.as_operation().to_string();
    let after = text.split("angle = ").nth(1).expect("angle attr");
    after
        .split(':')
        .next()
        .expect("angle token")
        .trim()
        .parse()
        .expect("parse angle")
}

fn read_func_depth(module: &Module<'_>) -> DepthExpr {
    let func = module.body().first_operation().expect("func");
    let attr = func.attribute(qc::attr::DEPTH).expect("depth");
    let text = StringAttribute::try_from(attr)
        .expect("depth string")
        .value()
        .to_string();
    DepthExpr::parse(&text).expect("depth expr")
}

#[test]
fn rz_rz_merge_angles() {
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_rotation_func(
        &context,
        &[("Rz", 0.5), ("Rz", 0.3)],
        &DepthExpr::Nat(2),
    ));
    rotation_merging::run_on_module(&context, &module);
    assert_eq!(count_rotation(&module, "Rz"), 1);
    assert!((read_angle(&module) - 0.8).abs() < 1e-6);
}

#[test]
fn rz_pi_pi_eliminates() {
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_rotation_func(
        &context,
        &[("Rz", PI), ("Rz", PI)],
        &DepthExpr::Nat(2),
    ));
    rotation_merging::run_on_module(&context, &module);
    assert_eq!(count_rotation(&module, "Rz"), 0);
    assert_eq!(read_func_depth(&module), DepthExpr::Nat(0));
}

#[test]
fn rz_rx_not_merged() {
    let context = context();
    let module = Module::new(Location::unknown(&context));
    module.body().append_operation(build_rotation_func(
        &context,
        &[("Rz", 0.5), ("Rx", 0.3)],
        &DepthExpr::Nat(2),
    ));
    rotation_merging::run_on_module(&context, &module);
    assert_eq!(count_rotation(&module, "Rz"), 1);
    assert_eq!(count_rotation(&module, "Rx"), 1);
}

#[test]
fn rz_h_rz_not_merged() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location)]);
    let mut wire = Value::from(block.argument(0).expect("arg"));
    wire = Value::from(
        block
            .append_operation(
                qc::rotation_gate(&context, "Rz", 0.5, 1, false, wire, location).expect("rz"),
            )
            .result(0)
            .expect("rz out"),
    );
    wire = Value::from(
        block
            .append_operation(qc::gate(&context, "H", 1, true, &[wire], location).expect("h"))
            .result(0)
            .expect("h out"),
    );
    wire = Value::from(
        block
            .append_operation(
                qc::rotation_gate(&context, "Rz", 0.3, 1, false, wire, location).expect("rz2"),
            )
            .result(0)
            .expect("rz2 out"),
    );
    block.append_operation(qc::r#return(&[wire], location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "main",
        1,
        1,
        &DepthExpr::Nat(3),
        false,
        region,
        location,
    )
    .expect("func");
    let module = Module::new(location);
    module.body().append_operation(func);
    rotation_merging::run_on_module(&context, &module);
    assert_eq!(count_rotation(&module, "Rz"), 2);
}
