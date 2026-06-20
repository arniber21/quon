//! Shared helpers for the `mlir_bridge` integration tests.

#![allow(dead_code)]

use melior::ir::attribute::{BoolAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationBuilder;
use melior::ir::r#type::IntegerType;
use melior::ir::{
    Attribute, Block, BlockLike, Identifier, Location, Module, Operation, Region, RegionLike, Type,
    Value,
};
use melior::Context;

use mlir_bridge::dialect::depth::DepthExpr;
use mlir_bridge::dialect::quantum_circ as qc;

/// A context with the `quantum.circ` dialect registered.
pub fn context() -> Context {
    let context = Context::new();
    qc::register_dialect(&context);
    context
}

pub fn i64_attr(context: &Context, value: i64) -> Attribute<'_> {
    IntegerAttribute::new(IntegerType::new(context, 64).into(), value).into()
}

pub fn str_attr<'c>(context: &'c Context, value: &str) -> Attribute<'c> {
    StringAttribute::new(context, value).into()
}

pub fn bool_attr(context: &Context, value: bool) -> Attribute<'_> {
    BoolAttribute::new(context, value).into()
}

/// A serialized depth attribute (a string, per ADR-0002).
pub fn depth_attr<'c>(context: &'c Context, depth: &DepthExpr) -> Attribute<'c> {
    str_attr(context, &depth.to_sexpr())
}

/// A detached block whose arguments source SSA values of the requested types.
/// Keep the returned block alive for as long as the values are used.
pub fn scratch_block<'c>(types: &[Type<'c>], location: Location<'c>) -> Block<'c> {
    let args: Vec<(Type, Location)> = types.iter().map(|t| (*t, location)).collect();
    Block::new(&args)
}

/// Builds an op in MLIR's generic form **without** running the dialect verifier.
/// Used to construct deliberately-malformed ops for verifier tests.
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
pub fn empty_body() -> Region<'static> {
    let region = Region::new();
    region.append_block(Block::new(&[]));
    region
}

/// Builds `func @main(%q: !qubit) -> !qubit { %r = gate "H" %q; return %r }`.
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
