//! Measurement deferral pass (issue #22, SPEC §7.3).
//!
//! Rewrites `measure` + single-use `if` patterns into a `unitary_region` with
//! classically-controlled corrections followed by a terminal measurement.

use std::collections::HashMap;

use melior::ir::attribute::{BoolAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{Block, BlockLike, Location, OperationRef, Region, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
use quon_core::DepthExpr;
use thiserror::Error;

use crate::dialect::{quantum_circ, quantum_dynamic};

#[derive(Debug, Error)]
pub enum DeferError {
    #[error("failed to build `{op}`: {message}")]
    Build { op: &'static str, message: String },
}

fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
    value.to_raw().ptr as usize
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct GateStep {
    name: String,
    targets: Vec<usize>,
    depth_contribution: i64,
    clifford: bool,
}

fn extract_gates_from_region<'c, 'a>(
    region: melior::ir::RegionRef<'c, 'a>,
    target_offset: usize,
) -> Vec<GateStep> {
    let mut gates = Vec::new();
    let Some(block) = region.first_block() else {
        return gates;
    };
    let mut op = block.first_operation();
    while let Some(inner) = op {
        let name = op_name(&inner);
        if name == quantum_circ::op::GATE {
            let gate_name = read_string_attr(&inner, quantum_circ::attr::GATE_NAME)
                .unwrap_or_else(|| "?".to_string());
            let depth_contribution =
                read_i64_attr(&inner, quantum_circ::attr::DEPTH_CONTRIBUTION).unwrap_or(1);
            let clifford = read_bool_attr(&inner, quantum_circ::attr::CLIFFORD).unwrap_or(true);
            let targets: Vec<usize> = inner
                .operands()
                .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
                .enumerate()
                .map(|(index, _)| index + target_offset)
                .collect();
            gates.push(GateStep {
                name: gate_name,
                targets,
                depth_contribution,
                clifford,
            });
        }
        op = inner.next_in_block();
    }
    gates
}

fn invert_clifford_gate(name: &str) -> Option<String> {
    match name {
        "X" | "Y" | "Z" | "H" | "CNOT" | "CZ" | "CX" | "SWAP" => Some(name.to_string()),
        "S" => Some("S†".to_string()),
        "S†" | "Sdag" => Some("S".to_string()),
        "T" => Some("T†".to_string()),
        "T†" | "Tdag" => Some("T".to_string()),
        _ => None,
    }
}

/// Builds the gate list whose composite operator is `U · V†` (then = `U`,
/// else = `V`), in circuit order (first-applied first).
///
/// The deferred region applies `V` unconditionally, then the controlled
/// correction. The deferred-measurement identity requires that correction to be
/// `C(U V†)`: `W = |0><0|⊗V + |1><1|⊗U = C(U V†) · (I⊗V)`. A circuit's operator
/// is the reverse product of its gates, so to realise the operator `U V†` we
/// emit `V†` first (the else gates reversed and inverted) followed by `U` (the
/// then gates in order).
fn compose_u_v_dagger(then_gates: &[GateStep], else_gates: &[GateStep]) -> Vec<GateStep> {
    let mut composed = Vec::with_capacity(then_gates.len() + else_gates.len());
    for gate in else_gates.iter().rev() {
        if let Some(inv) = invert_clifford_gate(&gate.name) {
            composed.push(GateStep {
                name: inv,
                targets: gate.targets.clone(),
                depth_contribution: gate.depth_contribution,
                clifford: gate.clifford,
            });
        }
    }
    composed.extend(then_gates.iter().cloned());
    composed
}

fn controlled_clifford_gate(
    control_index: usize,
    gate: &GateStep,
) -> Result<(String, Vec<usize>), DeferError> {
    if gate.targets.len() != 1 {
        return Err(DeferError::Build {
            op: quantum_dynamic::op::UNITARY_REGION,
            message: format!("unsupported multi-qubit branch gate `{}`", gate.name),
        });
    }
    let target = gate.targets[0];
    if target == control_index {
        return Err(DeferError::Build {
            op: quantum_dynamic::op::UNITARY_REGION,
            message: "branch gate cannot act on the measured control qubit".to_string(),
        });
    }
    match gate.name.as_str() {
        "X" => Ok(("CNOT".to_string(), vec![control_index, target])),
        "Z" => Ok(("CZ".to_string(), vec![control_index, target])),
        "I" | "identity" => Err(DeferError::Build {
            op: quantum_dynamic::op::UNITARY_REGION,
            message: "identity controlled gate".to_string(),
        }),
        other => Err(DeferError::Build {
            op: quantum_dynamic::op::UNITARY_REGION,
            message: format!("unsupported controlled branch gate `{other}`"),
        }),
    }
}

fn build_deferred_region<'c>(
    context: &'c Context,
    location: Location<'c>,
    control_index: usize,
    else_gates: &[GateStep],
    u_v_dagger: &[GateStep],
    num_qubits: usize,
) -> Result<Region<'c>, DeferError> {
    let qubit = quantum_circ::qubit_type(context);
    let block = Block::new(&vec![(qubit, location); num_qubits]);
    let mut wires: Vec<Value<'c, '_>> = Vec::with_capacity(num_qubits);
    for index in 0..num_qubits {
        let argument = block.argument(index).map_err(|_| DeferError::Build {
            op: quantum_dynamic::op::UNITARY_REGION,
            message: format!("missing block argument #{index}"),
        })?;
        wires.push(Value::from(argument));
    }

    for gate in else_gates {
        let operands: Vec<Value<'c, '_>> = gate.targets.iter().map(|index| wires[*index]).collect();
        let op = block.append_operation(
            quantum_circ::gate(context, &gate.name, 1, true, &operands, location).map_err(
                |error| DeferError::Build {
                    op: quantum_circ::op::GATE,
                    message: error.to_string(),
                },
            )?,
        );
        for (index, target) in gate.targets.iter().enumerate() {
            wires[*target] = Value::from(op.result(index).map_err(|_| DeferError::Build {
                op: quantum_circ::op::GATE,
                message: "missing gate result".to_string(),
            })?);
        }
    }
    for gate in u_v_dagger {
        if gate.name == "I" || gate.name == "identity" {
            continue;
        }
        let (name, targets) = controlled_clifford_gate(control_index, gate)?;
        let operands: Vec<Value<'c, '_>> = targets.iter().map(|index| wires[*index]).collect();
        let op = block.append_operation(
            quantum_circ::gate(context, &name, 1, true, &operands, location).map_err(|error| {
                DeferError::Build {
                    op: quantum_circ::op::GATE,
                    message: error.to_string(),
                }
            })?,
        );
        for (index, target) in targets.iter().enumerate() {
            wires[*target] = Value::from(op.result(index).map_err(|_| DeferError::Build {
                op: quantum_circ::op::GATE,
                message: "missing gate result".to_string(),
            })?);
        }
    }

    let outputs: Vec<Value<'c, '_>> = wires;
    block.append_operation(quantum_circ::r#return(&outputs, location).map_err(|error| {
        DeferError::Build {
            op: quantum_circ::op::RETURN,
            message: error.to_string(),
        }
    })?);
    let region = Region::new();
    region.append_block(block);
    Ok(region)
}

fn count_bit_uses<'c, 'a>(body: melior::ir::BlockRef<'c, 'a>, bit_key: usize) -> usize {
    let mut uses = 0usize;
    let mut op = body.first_operation();
    while let Some(current) = op {
        for operand in current.operands() {
            if quantum_dynamic::is_bit_type(operand.r#type()) && value_key(&operand) == bit_key {
                uses += 1;
            }
        }
        op = current.next_in_block();
    }
    uses
}

fn find_unique_if_for_bit<'c, 'a>(
    body: melior::ir::BlockRef<'c, 'a>,
    bit_key: usize,
) -> Option<OperationRef<'c, 'a>> {
    let mut found = None;
    let mut op = body.first_operation();
    while let Some(current) = op {
        if op_name(&current) == quantum_dynamic::op::IF
            && let Ok(condition) = current.operand(0)
            && value_key(&condition) == bit_key
        {
            if found.is_some() {
                return None;
            }
            found = Some(current);
        }
        op = current.next_in_block();
    }
    found
}

fn qubit_operands<'c, 'a>(operation: OperationRef<'c, 'a>) -> Vec<Value<'c, 'a>> {
    operation
        .operands()
        .skip(1)
        .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
        .collect()
}

fn defer_one<'c, 'a>(
    context: &'c Context,
    body: melior::ir::BlockRef<'c, 'a>,
    measure_op: OperationRef<'c, 'a>,
    if_op: OperationRef<'c, 'a>,
) -> Result<(), DeferError> {
    let location = measure_op.location();
    let q_measured = measure_op.operand(0).map_err(|_| DeferError::Build {
        op: quantum_dynamic::op::MEASURE,
        message: "missing measured qubit".to_string(),
    })?;
    let if_qubits = qubit_operands(if_op);
    let then_region = if_op.region(0).map_err(|_| DeferError::Build {
        op: quantum_dynamic::op::IF,
        message: "missing then region".to_string(),
    })?;
    let else_region = if_op.region(1).map_err(|_| DeferError::Build {
        op: quantum_dynamic::op::IF,
        message: "missing else region".to_string(),
    })?;
    let target_offset = 1usize;
    let then_gates = extract_gates_from_region(then_region, target_offset);
    let else_gates = extract_gates_from_region(else_region, target_offset);
    let u_v_dagger = compose_u_v_dagger(&then_gates, &else_gates);

    let mut operands = vec![q_measured];
    operands.extend(if_qubits.iter().copied());
    let control_index = 0usize;
    let region = build_deferred_region(
        context,
        location,
        control_index,
        &else_gates,
        &u_v_dagger,
        operands.len(),
    )?;

    let depth = DepthExpr::Nat(
        (else_gates
            .iter()
            .map(|g| g.depth_contribution.max(0) as u64)
            .sum::<u64>()
            + u_v_dagger.len() as u64)
            .max(1),
    );
    let deferred =
        quantum_dynamic::unitary_region(context, &operands, &depth, true, region, location)
            .map_err(|error| DeferError::Build {
                op: quantum_dynamic::op::UNITARY_REGION,
                message: error.to_string(),
            })?;

    let deferred_ref = body.insert_operation_before(if_op, deferred);
    let mut output_map: HashMap<usize, Value<'c, 'a>> = HashMap::new();
    for (index, result) in deferred_ref.results().enumerate() {
        output_map.insert(index, Value::from(result));
    }
    for (index, _qubit) in if_qubits.iter().enumerate() {
        if let Some(mapped) = output_map.get(&(index + 1))
            && let Ok(if_result) = if_op.result(index)
        {
            let rewriter = IrRewriter::new(context);
            rewriter
                .as_rewriter_base()
                .replace_all_uses_with(Value::from(if_result), *mapped);
        }
    }

    let measured_out = output_map.get(&0).copied().ok_or(DeferError::Build {
        op: quantum_dynamic::op::UNITARY_REGION,
        message: "deferred region has no control-qubit result".to_string(),
    })?;
    let terminal = quantum_dynamic::measure(context, measured_out, location).map_err(|error| {
        DeferError::Build {
            op: quantum_dynamic::op::MEASURE,
            message: error.to_string(),
        }
    })?;
    body.insert_operation_after(deferred_ref, terminal);

    let rewriter = IrRewriter::new(context);
    let base = rewriter.as_rewriter_base();
    base.erase_op(if_op);
    base.erase_op(measure_op);
    Ok(())
}

fn defer_module<'c, 'a>(context: &'c Context, module: OperationRef<'c, 'a>) {
    let Some(body) = module
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return;
    };

    let mut measures: Vec<OperationRef<'c, 'a>> = Vec::new();
    let mut op = body.first_operation();
    while let Some(current) = op {
        if op_name(&current) == quantum_dynamic::op::MEASURE {
            measures.push(current);
        }
        op = current.next_in_block();
    }

    for measure_op in measures.into_iter().rev() {
        let Ok(bit) = measure_op.result(0) else {
            continue;
        };
        let bit_value = Value::from(bit);
        let bit_key = value_key(&bit_value);
        if count_bit_uses(body, bit_key) != 1 {
            continue;
        }
        let Some(if_op) = find_unique_if_for_bit(body, bit_key) else {
            continue;
        };
        if let Err(error) = defer_one(context, body, measure_op, if_op) {
            eprintln!("measurement-deferral: {error}");
        }
    }
}

/// Runs measurement deferral on `module`.
pub fn run_on_module<'c>(context: &'c Context, module: &melior::ir::Module<'c>) {
    defer_module(context, module.as_operation());
}

#[repr(align(8))]
struct PassId;

static MEASUREMENT_DEFERRAL_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct MeasurementDeferral {
    context: usize,
}

impl MeasurementDeferral {
    fn new() -> Self {
        Self { context: 0 }
    }
}

impl<'c> RunExternalPass<'c> for MeasurementDeferral {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        if self.context == 0 {
            pass.signal_failure();
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        defer_module(context, operation);
    }
}

/// Creates the measurement deferral pass.
pub fn create_pass() -> Pass {
    create_external(
        MeasurementDeferral::new(),
        TypeId::create(&MEASUREMENT_DEFERRAL_PASS_ID),
        "measurement-deferral",
        "measurement-deferral",
        "Defer mid-circuit measurements whose bits feed a single terminal if",
        "",
        &[],
    )
}
