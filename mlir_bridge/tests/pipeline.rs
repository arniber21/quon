//! Smoke tests: Fixed pipeline stages are callable without `quonc` (#210).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::sabre_routing::SabreCost;
use mlir_bridge::pipeline::{run_circ_passes_to_fixpoint, run_dynamic_passes, run_fixed_physical};
use quon_core::DepthExpr;

use support::context;

fn hh_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location)]);
    let mut wire = Value::from(block.argument(0).expect("arg"));
    for _ in 0..2 {
        let op = block
            .append_operation(qc::gate(context, "H", 1, true, &[wire], location).expect("gate"));
        wire = Value::from(op.result(0).expect("result"));
    }
    block.append_operation(qc::r#return(&[wire], location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        context,
        "main",
        1,
        1,
        &DepthExpr::Nat(2),
        true,
        region,
        location,
    )
    .expect("func");
    let module = Module::new(location);
    module.body().append_operation(func);
    module
}

#[test]
fn circ_fixpoint_cancels_hh_without_quonc() {
    let context = context();
    let module = hh_module(&context);
    run_circ_passes_to_fixpoint(&context, &module);
    let text = module.as_operation().to_string();
    assert!(
        !text.contains("gate_name = \"H\""),
        "expected H·H cancelled via pipeline fixpoint: {text}"
    );
}

#[test]
fn fixed_physical_runs_on_emptyish_module() {
    // Dynamic + physical on a circ-only module: lowering is out of scope
    // here; just ensure the physical orchestration entry point is callable.
    let context = context();
    let module = Module::new(Location::unknown(&context));
    run_dynamic_passes(&context, &module);
    let target = backend::generic_openqasm::target(4);
    let _ = run_fixed_physical(&context, &target, SabreCost::default(), &module);
}
