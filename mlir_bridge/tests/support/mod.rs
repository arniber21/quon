//! Shared helpers for the `mlir_bridge` integration tests.

#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use melior::pass::{Pass, PassManager};
use melior::Context;
use melior::ir::attribute::{BoolAttribute, FloatAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationBuilder;
use melior::ir::operation::OperationLike;
use melior::ir::r#type::IntegerType;
use melior::ir::{
    Attribute, Block, BlockLike, Identifier, Location, Module, Operation, Region, RegionLike, Type,
    Value,
};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::dialect::quantum_dynamic as qd;
use quon_core::DepthExpr;

/// A context with the `quantum.circ` dialect registered.
#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn context() -> Context {
    let context = Context::new();
    qc::register_dialect(&context);
    context
}

/// A context with both `quantum.circ` and `quantum.dynamic` registered.
#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn dynamic_context() -> Context {
    let context = Context::new();
    qc::register_dialect(&context);
    qd::register_dialect(&context);
    context
}

#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn i64_attr(context: &Context, value: i64) -> Attribute<'_> {
    IntegerAttribute::new(IntegerType::new(context, 64).into(), value).into()
}

#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn str_attr<'c>(context: &'c Context, value: &str) -> Attribute<'c> {
    StringAttribute::new(context, value).into()
}

#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn bool_attr(context: &Context, value: bool) -> Attribute<'_> {
    BoolAttribute::new(context, value).into()
}

#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn i32_attr(context: &Context, value: i32) -> Attribute<'_> {
    IntegerAttribute::new(IntegerType::new(context, 32).into(), i64::from(value)).into()
}

#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn f64_attr(context: &Context, value: f64) -> Attribute<'_> {
    let float_type = Type::parse(context, "f64").unwrap_or_else(|| Type::none(context));
    FloatAttribute::new(context, float_type, value).into()
}

#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn f32_attr(context: &Context, value: f64) -> Attribute<'_> {
    let float_type = Type::parse(context, "f32").unwrap_or_else(|| Type::none(context));
    FloatAttribute::new(context, float_type, value).into()
}

/// A serialized depth attribute (a string, per ADR-0002).
#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn depth_attr<'c>(context: &'c Context, depth: &DepthExpr) -> Attribute<'c> {
    str_attr(context, &depth.to_sexpr())
}

/// A detached block whose arguments source SSA values of the requested types.
/// Keep the returned block alive for as long as the values are used.
#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn scratch_block<'c>(types: &[Type<'c>], location: Location<'c>) -> Block<'c> {
    let args: Vec<(Type, Location)> = types.iter().map(|t| (*t, location)).collect();
    Block::new(&args)
}

/// Appends a verified `quantum.dynamic.alloc` op producing one fresh
/// `!quantum.qubit` (issue #401 — replaces the unregistered `test.qubit`).
/// Appends a foreign op to `body` that produces one `!quantum.qubit`.
#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn append_foreign_qubit<'c: 'a, 'a, B: BlockLike<'c, 'a>>(
    context: &'c Context,
    body: &B,
    location: Location<'c>,
) -> Value<'c, 'a> {
    Value::from(
        body.append_operation(qd::alloc(context, 1, location).expect("alloc op"))
            .result(0)
            .expect("alloc qubit result"),
    )
}

/// The module's top-level region — the linearity scope for dynamic tests.
#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn module_region<'c>(module: &'c Module<'c>) -> melior::ir::RegionRef<'c, 'c> {
    module.as_operation().region(0).expect("module region")
}

/// Builds an op in MLIR's generic form **without** running the dialect verifier.
/// Used to construct deliberately-malformed ops for verifier tests.
#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn generic_op<'c>(
    context: &'c Context,
    name: &str,
    operands: &[Value<'c, '_>],
    results: &[Type<'c>],
    attributes: &[(&str, Attribute<'c>)],
    regions: Vec<Region<'c>>,
    location: Location<'c>,
) -> Operation<'c> {
    let attributes: Vec<(Identifier, Attribute)> = attributes
        .iter()
        .map(|(name, value)| (Identifier::new(context, name), *value))
        .collect();
    OperationBuilder::new(name, location)
        .add_operands(operands)
        .add_results(results)
        .add_attributes(&attributes)
        .add_regions_vec(regions)
        .build()
        .expect("generic op builds")
}

/// A region containing a single empty block — a minimal well-formed body.
#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn empty_body() -> Region<'static> {
    let region = Region::new();
    region.append_block(Block::new(&[]));
    region
}

/// Builds `func @main(%q: !qubit) -> !qubit { %r = gate "H" %q; return %r }`.
#[allow(dead_code)] // shared test helper — not every integration test uses every helper
pub fn bell_like_module(context: &Context) -> Module<'_> {
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

/// Runs `pass` on `module` through a fresh pass manager, capturing any
/// diagnostics emitted via `mlirEmitError` (i.e. via `Diagnostics::emit`)
/// into the returned vec.
///
/// `verify` toggles the pass manager's built-in verifier. Disable it for
/// passes run on deliberately-malformed IR so the recorded failure is the
/// pass's own logic, not MLIR verification rejecting the input up front.
pub fn run_pass_capturing_diagnostics(
    context: &Context,
    module: &mut Module<'_>,
    pass: Pass,
    verify: bool,
) -> (Result<(), melior::Error>, Vec<String>) {
    let captured: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let captured_handle = captured.clone();
    let id = context.attach_diagnostic_handler(move |diagnostic| {
        captured_handle.borrow_mut().push(diagnostic.to_string());
        true
    });
    let pass_manager = PassManager::new(context);
    pass_manager.enable_verifier(verify);
    pass_manager.add_pass(pass);
    let result = pass_manager.run(module);
    context.detach_diagnostic_handler(id);
    (result, captured.borrow().clone())
}
