//! ZX simplification pass tests (issue #20).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::zx_simplification;
use quon_core::DepthExpr;
use zx::simplify;

use support::context;

#[test]
fn zx_rewrite_fuses_double_rz_extracted_from_func() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let block = Block::new(&[(qubit, location)]);
    let mut wire = Value::from(block.argument(0).expect("arg"));
    for angle in [0.5, 0.3] {
        let op = block.append_operation(
            qc::rotation_gate(&context, "Rz", angle, 1, false, wire, location).expect("rz"),
        );
        wire = Value::from(op.result(0).expect("out"));
    }
    block.append_operation(qc::r#return(&[wire], location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "main",
        1,
        1,
        &DepthExpr::Nat(2),
        false,
        region,
        location,
    )
    .expect("func");
    let module = Module::new(location);
    let func_ref = module.body().append_operation(func);
    let gates = zx_simplification::extract_gates(func_ref);
    assert_eq!(gates.len(), 2);
    let mut zx = zx::circuit_to_zx(&gates);
    assert!(simplify(&mut zx) > 0);
    let _ = zx_simplification::simplify_func(&context, func_ref);
}
