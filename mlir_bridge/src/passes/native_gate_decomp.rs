//! Native gate decomposition pass (issue #24, SPEC §7.4).
//!
//! Decomposes non-native `quantum.circ.gate` ops into sequences of native gates
//! from the [`BackendTarget`], setting `native_gate=true` on all resulting ops.

use std::collections::HashMap;
use std::sync::Arc;

use backend::decompose::{decompose_named_single, decompose_named_two};
use backend::target::BackendTarget;
use melior::ir::attribute::{BoolAttribute, FloatAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{AttributeLike, BlockLike, Location, OperationRef, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
use mlir_sys::mlirOperationSetAttributeByName;
use melior::StringRef;
use thiserror::Error;

use crate::dialect::quantum_circ::{self, attr};

#[derive(Debug, Error)]
pub enum DecompError {
    #[error("failed to build `{op}`: {message}")]
    Build { op: &'static str, message: String },
}

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn read_string_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    key: &str,
) -> Option<String> {
    let value = operation.attribute(key).ok()?;
    StringAttribute::try_from(value)
        .ok()
        .map(|string| string.value().to_string())
}

fn read_i64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<i64> {
    let value = operation.attribute(key).ok()?;
    IntegerAttribute::try_from(value)
        .ok()
        .map(|integer| integer.value())
}

fn read_bool_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<bool> {
    let value = operation.attribute(key).ok()?;
    BoolAttribute::try_from(value).ok().map(|b| b.value())
}

fn read_f64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<f64> {
    let value = operation.attribute(key).ok()?;
    FloatAttribute::try_from(value).ok().map(|f| f.value())
}

fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
    value.to_raw().ptr as usize
}

fn native_gate_names(target: &BackendTarget) -> Vec<String> {
    target.native_gates.iter().map(|g| g.name.clone()).collect()
}

fn set_native_gate<'c>(context: &'c Context, op: OperationRef<'c, '_>, native: bool) {
    let attribute: melior::ir::Attribute<'_> = BoolAttribute::new(context, native).into();
    unsafe {
        mlirOperationSetAttributeByName(
            op.to_raw(),
            StringRef::new("native_gate").to_raw(),
            attribute.to_raw(),
        );
    }
}

fn append_native_gate<'c, 'a>(
    context: &'c Context,
    block: melior::ir::BlockRef<'c, 'a>,
    before: Option<OperationRef<'c, 'a>>,
    name: &str,
    depth: i64,
    clifford: bool,
    qubits: &[Value<'c, 'a>],
    angle: Option<f64>,
    location: Location<'c>,
) -> Result<OperationRef<'c, 'a>, DecompError> {
    let mut builder = melior::ir::operation::OperationBuilder::new(quantum_circ::op::GATE, location);
    builder = builder.add_operands(qubits);
    let results = vec![quantum_circ::qubit_type(context); qubits.len()];
    builder = builder.add_results(&results);
    let mut attrs = vec![
        (
            melior::ir::Identifier::new(context, attr::GATE_NAME),
            StringAttribute::new(context, name).into(),
        ),
        (
            melior::ir::Identifier::new(context, attr::DEPTH_CONTRIBUTION),
            IntegerAttribute::new(
                melior::ir::r#type::IntegerType::new(context, 64).into(),
                depth,
            )
            .into(),
        ),
        (
            melior::ir::Identifier::new(context, attr::CLIFFORD),
            BoolAttribute::new(context, clifford).into(),
        ),
        (
            melior::ir::Identifier::new(context, "native_gate"),
            BoolAttribute::new(context, true).into(),
        ),
    ];
    if let Some(theta) = angle {
        let float_type = melior::ir::Type::parse(context, "f64").unwrap_or_else(|| melior::ir::Type::none(context));
        attrs.push((
            melior::ir::Identifier::new(context, attr::ANGLE),
            FloatAttribute::new(context, float_type, theta).into(),
        ));
    }
    builder = builder.add_attributes(&attrs);
    let built = builder.build().map_err(|error| DecompError::Build {
        op: quantum_circ::op::GATE,
        message: error.to_string(),
    })?;
    quantum_circ::verify(&built).map_err(|error| DecompError::Build {
        op: quantum_circ::op::GATE,
        message: error.to_string(),
    })?;
    let op_ref = if let Some(anchor) = before {
        block.insert_operation_before(anchor, built)
    } else {
        block.append_operation(built)
    };
    Ok(op_ref)
}

fn decompose_gate<'c, 'a>(
    context: &'c Context,
    target: &BackendTarget,
    block: melior::ir::BlockRef<'c, 'a>,
    gate: OperationRef<'c, 'a>,
) -> Result<(), DecompError> {
    if read_bool_attr(&gate, "native_gate").unwrap_or(false) {
        return Ok(());
    }

    let gate_name = read_string_attr(&gate, attr::GATE_NAME).unwrap_or_default();
    let depth = read_i64_attr(&gate, attr::DEPTH_CONTRIBUTION).unwrap_or(1);
    let clifford = read_bool_attr(&gate, attr::CLIFFORD).unwrap_or(true);
    let angle = read_f64_attr(&gate, attr::ANGLE);
    let location = gate.location();

    let native_names = native_gate_names(target);
    let qubits: Vec<Value<'c, 'a>> = gate
        .operands()
        .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
        .collect();

    if target.is_native(&gate_name) {
        set_native_gate(context, gate, true);
        return Ok(());
    }

    let decomposed = if qubits.len() == 1 {
        decompose_named_single(&gate_name, angle, &native_names, 0)
    } else if qubits.len() == 2 {
        decompose_named_two(&gate_name, &native_names, 0, 1)
    } else {
        return Err(DecompError::Build {
            op: quantum_circ::op::GATE,
            message: format!("unsupported gate arity {}", qubits.len()),
        });
    };

    if decomposed.is_empty() {
        return Err(DecompError::Build {
            op: quantum_circ::op::GATE,
            message: format!("no decomposition for `{gate_name}`"),
        });
    }

    let mut wires: HashMap<usize, Value<'c, 'a>> = qubits
        .iter()
        .enumerate()
        .map(|(index, value)| (index, *value))
        .collect();

    let mut last_results: Vec<Value<'c, 'a>> = qubits.clone();
    for step in &decomposed {
        let operands: Vec<Value<'c, 'a>> = step
            .qubits
            .iter()
            .map(|index| wires[index])
            .collect();
        let op_ref = append_native_gate(
            context,
            block,
            Some(gate),
            &step.name,
            depth,
            clifford,
            &operands,
            step.params.first().copied(),
            location,
        )?;
        last_results = op_ref
            .results()
            .map(Value::from)
            .collect();
        for (index, result) in step.qubits.iter().zip(last_results.iter()) {
            wires.insert(*index, *result);
        }
    }

    let rewriter = IrRewriter::new(context);
    let base = rewriter.as_rewriter_base();
    for (original, replacement) in gate.results().zip(last_results.iter()) {
        base.replace_all_uses_with(Value::from(original), *replacement);
    }
    base.erase_op(gate);
    Ok(())
}

fn decompose_module<'c, 'a>(context: &'c Context, target: &BackendTarget, module: OperationRef<'c, 'a>) {
    let Some(body) = module
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return;
    };

    let mut op = body.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        if op_name(&current) != quantum_circ::op::FUNC {
            continue;
        }
        let Ok(region) = current.region(0) else {
            continue;
        };
        let Some(block) = region.first_block() else {
            continue;
        };
        let mut inner = block.first_operation();
        while let Some(gate) = inner {
            inner = gate.next_in_block();
            if op_name(&gate) != quantum_circ::op::GATE {
                continue;
            }
            if let Err(error) = decompose_gate(context, target, block, gate) {
                eprintln!("native-gate-decomp: {error}");
            }
        }
    }
}

/// Runs native gate decomposition on `module` using `target`.
pub fn run_on_module<'c>(context: &'c Context, target: &BackendTarget, module: &melior::ir::Module<'c>) {
    decompose_module(context, target, module.as_operation());
}

#[repr(align(8))]
struct PassId;

static NATIVE_GATE_DECOMP_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct NativeGateDecomp {
    context: usize,
    target: Arc<BackendTarget>,
}

impl NativeGateDecomp {
    fn new(target: BackendTarget) -> Self {
        Self {
            context: 0,
            target: Arc::new(target),
        }
    }
}

impl<'c> RunExternalPass<'c> for NativeGateDecomp {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        if self.context == 0 {
            pass.signal_failure();
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        decompose_module(context, &self.target, operation);
    }
}

/// Creates the native gate decomposition pass for `target`.
pub fn create_pass(target: BackendTarget) -> Pass {
    create_external(
        NativeGateDecomp::new(target),
        TypeId::create(&NATIVE_GATE_DECOMP_PASS_ID),
        "native-gate-decomp",
        "native-gate-decomp",
        "Decompose non-native quantum.circ.gate ops into native gate sequences",
        "",
        &[],
    )
}
