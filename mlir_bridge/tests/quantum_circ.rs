//! Acceptance tests for issue #4: `quantum.circ` dialect registration, op
//! verifiers, and the linearity verifier pass.

use melior::ir::operation::OperationBuilder;
use melior::ir::r#type::IntegerType;
use melior::ir::Value;
use melior::ir::{Attribute, Block, BlockLike, Identifier, Location, Module, Region, RegionLike};
use melior::pass::PassManager;
use melior::Context;

use mlir_bridge::dialect::depth::DepthExpr;
use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::linearity_verifier;

fn context() -> Context {
    let context = Context::new();
    qc::register_dialect(&context);
    context
}

/// Builds `func @main(%q: !qubit) -> !qubit { %r = gate "H" %q; return %r }`.
fn bell_like_module(context: &Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);

    let block = Block::new(&[(qubit, location)]);
    let input = Value::from(block.argument(0).expect("entry argument"));

    let gate = qc::gate(context, "H", 1, true, &[input], location).expect("gate op");
    let gate = block.append_operation(gate);
    let output = Value::from(gate.result(0).expect("gate result"));

    let terminator = qc::r#return(&[output], location).expect("return op");
    block.append_operation(terminator);

    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        context,
        "main",
        1,
        1,
        &DepthExpr::Nat(1),
        true,
        region,
        location,
    )
    .expect("func op");

    let module = Module::new(location);
    module.body().append_operation(func);
    module
}

// --- Registration & round-trip --------------------------------------------

#[test]
fn registration_is_idempotent_and_panic_free() {
    let context = context();
    qc::register_dialect(&context);
    qc::register_dialect(&context);
    assert!(context.allow_unregistered_dialects());
    assert_eq!(qc::OPS.len(), 7);
}

#[test]
fn all_seven_ops_build_and_verify() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let circuit = qc::circuit_type(&context);
    let depth = DepthExpr::Nat(1);

    // A scratch block sourcing qubit and circuit SSA values for the builders.
    let block = Block::new(&[(qubit, location), (circuit, location), (circuit, location)]);
    let q = Value::from(block.argument(0).unwrap());
    let c0 = Value::from(block.argument(1).unwrap());
    let c1 = Value::from(block.argument(2).unwrap());

    // func (built in bell_like_module); the remaining six ops here.
    qc::gate(&context, "X", 1, true, &[q], location).expect("gate");
    qc::compose(&context, c0, c1, &depth, location).expect("compose");
    qc::tensor(&context, c0, c1, &depth, location).expect("tensor");
    qc::adjoint(&context, c0, &depth, location).expect("adjoint");
    qc::controlled(&context, c0, q, &depth, location).expect("controlled");

    let borrow_body = Region::new();
    borrow_body.append_block(Block::new(&[]));
    qc::borrow(&context, 1, &depth, borrow_body, location).expect("borrow");

    // func round-trips through a module.
    let _ = bell_like_module(&context);
}

#[test]
fn module_round_trips_through_mlir_text() {
    let context = context();
    let module = bell_like_module(&context);

    let text = module.as_operation().to_string();
    assert!(text.contains("quantum.circ.func"));
    assert!(text.contains("quantum.circ.gate"));
    assert!(text.contains("!quantum.qubit"));

    let reparsed = Module::parse(&context, &text).expect("re-parse module text");
    let reprinted = reparsed.as_operation().to_string();

    // quantum.circ module → MLIR text → re-parsed → identical.
    assert_eq!(text, reprinted);
}

// --- Verifier rejection ----------------------------------------------------

#[test]
fn verifier_rejects_gate_missing_attributes() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);

    let block = Block::new(&[(qubit, location)]);
    let q = Value::from(block.argument(0).unwrap());

    // No gate_name / depth_contribution attributes.
    let malformed = OperationBuilder::new(qc::op::GATE, location)
        .add_operands(&[q])
        .add_results(&[qubit])
        .build()
        .unwrap();

    assert!(matches!(
        qc::verify(&malformed),
        Err(qc::VerifyError::MissingAttribute { .. })
    ));
}

#[test]
fn verifier_rejects_gate_with_wrong_qubit_count() {
    let context = context();
    let location = Location::unknown(&context);
    let qubit = qc::qubit_type(&context);
    let i64_type = IntegerType::new(&context, 64).into();

    let block = Block::new(&[(qubit, location)]);
    let q = Value::from(block.argument(0).unwrap());

    // One operand but two results — not qubit-preserving.
    let malformed = OperationBuilder::new(qc::op::GATE, location)
        .add_operands(&[q])
        .add_results(&[qubit, qubit])
        .add_attributes(&[
            (
                Identifier::new(&context, qc::attr::GATE_NAME),
                Attribute::parse(&context, "\"H\"").unwrap(),
            ),
            (
                Identifier::new(&context, qc::attr::DEPTH_CONTRIBUTION),
                melior::ir::attribute::IntegerAttribute::new(i64_type, 1).into(),
            ),
        ])
        .build()
        .unwrap();

    assert!(matches!(
        qc::verify(&malformed),
        Err(qc::VerifyError::Arity { role: "result", .. })
    ));
}

#[test]
fn verifier_rejects_func_missing_attributes() {
    let context = context();
    let location = Location::unknown(&context);

    let region = Region::new();
    region.append_block(Block::new(&[]));
    let malformed = OperationBuilder::new(qc::op::FUNC, location)
        .add_regions([region])
        .build()
        .unwrap();

    assert!(matches!(
        qc::verify(&malformed),
        Err(qc::VerifyError::MissingAttribute { .. })
    ));
}

#[test]
fn verifier_accepts_well_formed_ops() {
    let context = context();
    let module = bell_like_module(&context);
    // Every op the builders produced already passed verify(); re-verify the
    // func reachable from the module body for good measure.
    let body = module.body();
    let func = body.first_operation().expect("func op");
    assert!(qc::verify(&func).is_ok());
}

// --- Linearity pass --------------------------------------------------------

/// Builds a single-input `func` whose body the caller fills in, runs the
/// linearity check, and asserts the diagnostic count. The linearity invariant
/// is independent of the op verifier, so a non-linear body still builds.
fn linear_check(
    out_qubits: i64,
    diagnostics_expected: usize,
    build: impl FnOnce(&Context, &Block, Location),
) {
    let context = context();
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

    let diagnostics = linearity_verifier::check_linearity(&func);
    assert_eq!(
        diagnostics.len(),
        diagnostics_expected,
        "diagnostics: {:?}",
        diagnostics.iter().map(|d| d.message()).collect::<Vec<_>>()
    );
}

#[test]
fn linearity_accepts_single_use() {
    linear_check(1, 0, |context, block, location| {
        let q = Value::from(block.argument(0).unwrap());
        let gate = qc::gate(context, "H", 1, true, &[q], location).unwrap();
        let gate = block.append_operation(gate);
        let r = Value::from(gate.result(0).unwrap());
        block.append_operation(qc::r#return(&[r], location).unwrap());
    });
}

#[test]
fn linearity_detects_unused_qubit() {
    // The input qubit is never used: 0 uses.
    linear_check(0, 1, |_context, block, location| {
        block.append_operation(qc::r#return(&[], location).unwrap());
    });
}

#[test]
fn linearity_detects_double_use() {
    // The input qubit feeds two gates: 2 uses.
    linear_check(2, 1, |context, block, location| {
        let q = Value::from(block.argument(0).unwrap());
        let g1 = qc::gate(context, "H", 1, true, &[q], location).unwrap();
        let g1 = block.append_operation(g1);
        let r1 = Value::from(g1.result(0).unwrap());
        let g2 = qc::gate(context, "X", 1, true, &[q], location).unwrap();
        let g2 = block.append_operation(g2);
        let r2 = Value::from(g2.result(0).unwrap());
        block.append_operation(qc::r#return(&[r1, r2], location).unwrap());
    });
}

#[test]
fn linearity_pass_runs_through_pass_manager() {
    let context = context();
    let mut module = bell_like_module(&context);

    let pass_manager = PassManager::new(&context);
    pass_manager.add_pass(linearity_verifier::create_pass());
    assert!(pass_manager.run(&mut module).is_ok());
}
