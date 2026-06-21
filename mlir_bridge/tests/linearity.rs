//! Linearity verifier tests for issue #4: every `!qubit` value in a
//! `quantum.circ.func` region must have exactly one use.

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::linearity_verifier::check_linearity;
use quon_core::DepthExpr;

/// Builds `func @main` with one qubit input, fills the body via `build`, runs
/// the linearity check, and asserts the diagnostic count.
fn check(out_qubits: i64, expected: usize, build: impl FnOnce(&melior::Context, &Block, Location)) {
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);

    let block = Block::new(&[(qubit, location)]);
    build(&context, &block, location);

    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "main",
        1,
        out_qubits,
        &DepthExpr::Nat(1),
        true,
        region,
        location,
    )
    .expect("func op");

    let module = Module::new(location);
    let func = module.body().append_operation(func);

    let diagnostics = check_linearity(&func);
    assert_eq!(
        diagnostics.len(),
        expected,
        "messages: {:?}",
        diagnostics
            .iter()
            .map(|d| d.message().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn accepts_single_use_chain() {
    check(1, 0, |context, block, location| {
        let q = Value::from(block.argument(0).unwrap());
        let g = block.append_operation(qc::gate(context, "H", 1, true, &[q], location).unwrap());
        let r = Value::from(g.result(0).unwrap());
        block.append_operation(qc::r#return(&[r], location).unwrap());
    });
}

#[test]
fn detects_unused_input() {
    // 0 uses of the input qubit.
    check(0, 1, |_context, block, location| {
        block.append_operation(qc::r#return(&[], location).unwrap());
    });
}

#[test]
fn detects_double_use() {
    // 2 uses of the input qubit.
    check(2, 1, |context, block, location| {
        let q = Value::from(block.argument(0).unwrap());
        let g1 = block.append_operation(qc::gate(context, "H", 1, true, &[q], location).unwrap());
        let r1 = Value::from(g1.result(0).unwrap());
        let g2 = block.append_operation(qc::gate(context, "X", 1, true, &[q], location).unwrap());
        let r2 = Value::from(g2.result(0).unwrap());
        block.append_operation(qc::r#return(&[r1, r2], location).unwrap());
    });
}

#[test]
fn detects_unused_intermediate() {
    // The gate result is never consumed: the intermediate has 0 uses.
    check(1, 1, |context, block, location| {
        let q = Value::from(block.argument(0).unwrap());
        block.append_operation(qc::gate(context, "H", 1, true, &[q], location).unwrap());
        // Return nothing; out_qubits is irrelevant to the linearity check.
    });
}

#[test]
fn reports_one_diagnostic_per_violation() {
    // Input used twice (1 violation) AND a fresh unused gate result (1 more).
    check(2, 2, |context, block, location| {
        let q = Value::from(block.argument(0).unwrap());
        let g1 = block.append_operation(qc::gate(context, "H", 1, true, &[q], location).unwrap());
        let r1 = Value::from(g1.result(0).unwrap());
        // Reuse q (violation 1); g2's result r2 is returned, but a third gate's
        // result is dropped (violation 2).
        let g2 = block.append_operation(qc::gate(context, "X", 1, true, &[q], location).unwrap());
        let r2 = Value::from(g2.result(0).unwrap());
        block.append_operation(qc::gate(context, "Z", 1, true, &[r1], location).unwrap());
        block.append_operation(qc::r#return(&[r2], location).unwrap());
    });
}

#[test]
fn accepts_two_qubit_gate() {
    // A CX threads two qubits; each input and each result is used exactly once.
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);

    let block = Block::new(&[(qubit, location), (qubit, location)]);
    let q0 = Value::from(block.argument(0).unwrap());
    let q1 = Value::from(block.argument(1).unwrap());
    let cx =
        block.append_operation(qc::gate(&context, "CX", 1, true, &[q0, q1], location).unwrap());
    let r0 = Value::from(cx.result(0).unwrap());
    let r1 = Value::from(cx.result(1).unwrap());
    block.append_operation(qc::r#return(&[r0, r1], location).unwrap());

    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "main",
        2,
        2,
        &DepthExpr::Nat(1),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    let func = module.body().append_operation(func);

    assert!(check_linearity(&func).is_empty());
}

#[test]
fn ignores_circuit_valued_ssa() {
    // Circuit values are not qubits: a circuit result used twice (or zero times)
    // is fine as far as linearity is concerned.
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let circuit = qc::circuit_type(&context);

    let block = Block::new(&[(qubit, location)]);
    let q = Value::from(block.argument(0).unwrap());
    // A foreign op produces a circuit value that is never consumed.
    block.append_operation(support::generic_op(
        &context,
        "test.circuit",
        &[],
        &[circuit],
        &[],
        vec![],
        location,
    ));
    let g = block.append_operation(qc::gate(&context, "H", 1, true, &[q], location).unwrap());
    let r = Value::from(g.result(0).unwrap());
    block.append_operation(qc::r#return(&[r], location).unwrap());

    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "main",
        1,
        1,
        &DepthExpr::Nat(1),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    let func = module.body().append_operation(func);

    // The unused circuit value must NOT be flagged.
    assert!(check_linearity(&func).is_empty());
}

#[test]
fn descends_into_borrow_region() {
    // A qubit defined as a borrow-body block argument and used twice inside the
    // nested region must still be caught.
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);

    // Outer func body: input qubit returned once (linear at the top level).
    let outer = Block::new(&[(qubit, location)]);
    let q = Value::from(outer.argument(0).unwrap());

    // borrow { ^bb(%a: !qubit): gate, gate (uses %a twice) }
    let inner = Block::new(&[(qubit, location)]);
    let a = Value::from(inner.argument(0).unwrap());
    inner.append_operation(qc::gate(&context, "H", 1, true, &[a], location).unwrap());
    inner.append_operation(qc::gate(&context, "X", 1, true, &[a], location).unwrap());
    let inner_region = Region::new();
    inner_region.append_block(inner);
    outer.append_operation(
        qc::borrow(&context, 1, &DepthExpr::Nat(1), inner_region, location).unwrap(),
    );

    outer.append_operation(qc::r#return(&[q], location).unwrap());
    let region = Region::new();
    region.append_block(outer);
    let func = qc::func(
        &context,
        "main",
        1,
        1,
        &DepthExpr::Nat(1),
        true,
        region,
        location,
    )
    .unwrap();
    let module = Module::new(location);
    let func = module.body().append_operation(func);

    let diagnostics = check_linearity(&func);
    let messages: Vec<String> = diagnostics
        .iter()
        .map(|d| d.message().to_string())
        .collect();
    // Inside the borrow region: %a has 2 uses (1 error) and the two gate results
    // are each unused (2 errors). Plus the borrow's own result qubit is unused at
    // the outer level (1 error). The outer input q is returned once (ok). = 4.
    assert_eq!(diagnostics.len(), 4, "messages: {messages:?}");
    // The double-use inside the nested region was caught — proving recursion.
    assert!(
        messages.iter().any(|m| m.contains("2 use(s)")),
        "messages: {messages:?}"
    );
}

#[test]
fn pass_succeeds_on_linear_module() {
    let context = support::context();
    let mut module = support::bell_like_module(&context);

    let pass_manager = melior::pass::PassManager::new(&context);
    pass_manager.add_pass(mlir_bridge::passes::linearity_verifier::create_pass());
    assert!(pass_manager.run(&mut module).is_ok());
}

#[test]
fn pass_fails_on_nonlinear_module() {
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);

    let block = Block::new(&[(qubit, location)]);
    // Input qubit dropped: 0 uses.
    block.append_operation(qc::r#return(&[], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        &context,
        "main",
        1,
        0,
        &DepthExpr::Nat(0),
        true,
        region,
        location,
    )
    .unwrap();
    let mut module = Module::new(location);
    module.body().append_operation(func);

    let pass_manager = melior::pass::PassManager::new(&context);
    pass_manager.add_pass(mlir_bridge::passes::linearity_verifier::create_pass());
    assert!(pass_manager.run(&mut module).is_err());
}

#[test]
fn pass_handles_module_without_funcs() {
    let context = support::context();
    let location = Location::unknown(&context);
    let mut module = Module::new(location);
    module.body().append_operation(support::generic_op(
        &context,
        "test.circuit",
        &[],
        &[qc::circuit_type(&context)],
        &[],
        vec![],
        location,
    ));

    let pass_manager = melior::pass::PassManager::new(&context);
    pass_manager.add_pass(mlir_bridge::passes::linearity_verifier::create_pass());
    assert!(pass_manager.run(&mut module).is_ok());
}

#[test]
fn checks_every_func_in_a_module() {
    // Two funcs, one linear and one not: the module-level walk finds the bad one.
    let context = support::context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let mut module = support::bell_like_module(&context);

    // Append a second, non-linear func (input dropped).
    let block = Block::new(&[(qubit, location)]);
    block.append_operation(qc::r#return(&[], location).unwrap());
    let region = Region::new();
    region.append_block(block);
    let bad = qc::func(
        &context,
        "bad",
        1,
        0,
        &DepthExpr::Nat(0),
        true,
        region,
        location,
    )
    .unwrap();
    module.body().append_operation(bad);

    let pass_manager = melior::pass::PassManager::new(&context);
    pass_manager.add_pass(mlir_bridge::passes::linearity_verifier::create_pass());
    assert!(pass_manager.run(&mut module).is_err());
}
