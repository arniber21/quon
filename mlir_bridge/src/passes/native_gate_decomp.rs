//! Native gate decomposition pass (issue #24, SPEC §7.4).
//!
//! Decomposes non-native `quantum.circ.gate` ops into sequences of native gates
//! from the [`BackendTarget`], setting `native_gate=true` on all resulting ops.

use std::collections::HashMap;
use std::sync::Arc;

use backend::decompose::{decompose_named_single, decompose_named_two};
use backend::target::{BackendTarget, FixedTarget};
use melior::StringRef;
use melior::ir::attribute::{BoolAttribute, FloatAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{AttributeLike, BlockLike, Location, OperationRef, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
use mlir_sys::mlirOperationSetAttributeByName;
use thiserror::Error;

use crate::dialect::quantum_circ::{self, attr};
use crate::dialect::quantum_dynamic;

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

fn native_gate_names(target: &FixedTarget) -> Vec<String> {
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

#[allow(clippy::too_many_arguments)]
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
    let mut builder =
        melior::ir::operation::OperationBuilder::new(quantum_circ::op::GATE, location);
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
        let float_type = melior::ir::Type::parse(context, "f64")
            .unwrap_or_else(|| melior::ir::Type::none(context));
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
    target: &FixedTarget,
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

    // `decompose_named_single` distinguishes "unrecognized gate" (`None`, a
    // real error) from "recognized but needs zero native gates" (`Some(vec![])`
    // — e.g. the identity — which must wire straight through, not error).
    // `decompose_named_two` cannot yet make that distinction, so an empty
    // two-qubit decomposition is still treated as an error.
    let decomposed = if qubits.len() == 1 {
        match decompose_named_single(&gate_name, angle, &native_names, 0) {
            Some(ops) => ops,
            None => {
                return Err(DecompError::Build {
                    op: quantum_circ::op::GATE,
                    message: format!("no decomposition for `{gate_name}`"),
                });
            }
        }
    } else if qubits.len() == 2 {
        let ops = decompose_named_two(&gate_name, &native_names, 0, 1);
        if ops.is_empty() {
            return Err(DecompError::Build {
                op: quantum_circ::op::GATE,
                message: format!("no decomposition for `{gate_name}`"),
            });
        }
        ops
    } else {
        return Err(DecompError::Build {
            op: quantum_circ::op::GATE,
            message: format!("unsupported gate arity {}", qubits.len()),
        });
    };

    let mut wires: HashMap<usize, Value<'c, 'a>> = qubits
        .iter()
        .enumerate()
        .map(|(index, value)| (index, *value))
        .collect();

    let mut last_results: Vec<Value<'c, 'a>> = qubits.clone();
    for step in &decomposed {
        let operands: Vec<Value<'c, 'a>> = step.qubits.iter().map(|index| wires[index]).collect();
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
        last_results = op_ref.results().map(Value::from).collect();
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

/// Decomposes every `quantum.circ.gate` reachable from `block`, recursing into
/// nested `quantum.dynamic.unitary_region` and `quantum.dynamic.if` bodies.
///
/// Decomposition is stateless per gate (no cross-region layout to thread), so a
/// plain recursive walk is sound here — unlike SABRE routing or depth
/// scheduling, which must track a qubit's physical identity *across* region
/// boundaries (see the analogous walk in `sabre_routing`/`depth_scheduling`).
///
/// This must reach the module's own top-level block, not just named
/// `quantum.circ.func`s: after lowering, the executed program's body
/// (originally the `main` run block) is bare top-level ops, not wrapped in a
/// func — only leftover, since-inlined circuit function *definitions* still
/// use `quantum.circ.func`.
fn decompose_block<'c, 'a>(
    context: &'c Context,
    target: &FixedTarget,
    block: melior::ir::BlockRef<'c, 'a>,
) {
    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        let name = op_name(&current);
        if name == quantum_circ::op::GATE {
            if let Err(error) = decompose_gate(context, target, block, current) {
                eprintln!("native-gate-decomp: {error}");
            }
        } else if name == quantum_dynamic::op::UNITARY_REGION {
            if let Ok(region) = current.region(0)
                && let Some(inner_block) = region.first_block()
            {
                decompose_block(context, target, inner_block);
            }
        } else if name == quantum_dynamic::op::IF {
            for region_index in 0..2 {
                if let Ok(region) = current.region(region_index)
                    && let Some(inner_block) = region.first_block()
                {
                    decompose_block(context, target, inner_block);
                }
            }
        }
    }
}

fn decompose_module<'c, 'a>(
    context: &'c Context,
    target: &FixedTarget,
    module: OperationRef<'c, 'a>,
) {
    let Some(body) = module
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return;
    };

    decompose_block(context, target, body);

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
        decompose_block(context, target, block);
    }
}

/// Runs native gate decomposition on `module` using `target`.
pub fn run_on_module<'c>(
    context: &'c Context,
    target: &BackendTarget,
    module: &melior::ir::Module<'c>,
) {
    let Some(target) = target.fixed_target() else {
        return;
    };
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
        let Some(target) = self.target.fixed_target() else {
            return;
        };
        let context = unsafe { &*(self.context as *const Context) };
        decompose_module(context, target, operation);
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
