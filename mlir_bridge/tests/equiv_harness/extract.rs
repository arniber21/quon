//! SSA-aware gate extraction from `quantum.circ.func` / `borrow` bodies.
//!
//! Unlike `zx_simplification::extract_gates`, this tracks logical qubit indices
//! through SSA wire values (block args → gate results), so multi-qubit circuits
//! extract correctly.

use std::collections::{HashMap, HashSet};

use melior::ir::attribute::{FloatAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, OperationRef, RegionLike, Value, ValueLike};

use mlir_bridge::dialect::quantum_circ::{self, attr};

use super::circuit_spec::{CircuitSpec, GateInst, GateKind};

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
    value.to_raw().ptr as usize
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

fn read_f64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<f64> {
    let value = operation.attribute(key).ok()?;
    FloatAttribute::try_from(value)
        .ok()
        .map(|float| float.value())
}

fn parse_gate_kind(name: &str) -> Option<GateKind> {
    match name {
        "H" | "h" => Some(GateKind::H),
        "X" | "x" => Some(GateKind::X),
        "Y" | "y" => Some(GateKind::Y),
        "Z" | "z" => Some(GateKind::Z),
        "S" | "s" => Some(GateKind::S),
        "S†" | "Sdag" | "sdg" => Some(GateKind::Sdag),
        "T" | "t" => Some(GateKind::T),
        "T†" | "Tdag" | "tdg" => Some(GateKind::Tdag),
        "CNOT" | "CX" | "cx" => Some(GateKind::CNOT),
        "CZ" | "cz" => Some(GateKind::CZ),
        "SWAP" | "swap" => Some(GateKind::SWAP),
        "Rx" | "rx" => Some(GateKind::Rx),
        "Ry" | "ry" => Some(GateKind::Ry),
        "Rz" | "rz" => Some(GateKind::Rz),
        _ => None,
    }
}

struct RecordedGate {
    op_ptr: usize,
    inst: GateInst,
    result_keys: Vec<usize>,
    operand_keys: Vec<usize>,
}

/// Extract gates from a region body, mapping SSA values to logical indices.
///
/// When `live_only` is true, only gates that feed the `return` (SSA use-def
/// cone) are kept — dead adjoints that never rewrite `return` are excluded.
fn extract_from_block<'c, 'a>(
    block: melior::ir::BlockRef<'c, 'a>,
    live_only: bool,
) -> Option<(Vec<GateInst>, Vec<u8>, u8)> {
    let width = block.argument_count() as u8;
    let mut wire_to_logical: HashMap<usize, u8> = HashMap::new();
    for i in 0..block.argument_count() {
        let arg = block.argument(i).ok()?;
        wire_to_logical.insert(value_key(&Value::from(arg)), i as u8);
    }

    let mut recorded = Vec::new();
    let mut return_operand_keys: Option<Vec<usize>> = None;
    let mut op = block.first_operation();

    while let Some(current) = op {
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN {
            let mut keys = Vec::with_capacity(current.operand_count());
            for i in 0..current.operand_count() {
                let operand = current.operand(i).ok()?;
                keys.push(value_key(&operand));
            }
            return_operand_keys = Some(keys);
            break;
        }
        if name == quantum_circ::op::GATE {
            let gate_name = read_string_attr(&current, attr::GATE_NAME)?;
            let kind = parse_gate_kind(&gate_name)?;
            let mut qubits = Vec::with_capacity(current.operand_count());
            let mut operand_keys = Vec::with_capacity(current.operand_count());
            for i in 0..current.operand_count() {
                let operand = current.operand(i).ok()?;
                operand_keys.push(value_key(&operand));
                let logical = *wire_to_logical.get(&value_key(&operand))?;
                qubits.push(logical);
            }
            let angle = read_f64_attr(&current, attr::ANGLE);
            // Positional: result i continues the same wire-slot label as operand i.
            // (SABRE layout is recovered separately by simulating SWAPs.)
            let mut result_keys = Vec::with_capacity(current.result_count());
            for (i, &logical) in qubits.iter().enumerate() {
                let result = current.result(i).ok()?;
                let key = value_key(&Value::from(result));
                result_keys.push(key);
                wire_to_logical.insert(key, logical);
            }
            recorded.push(RecordedGate {
                op_ptr: current.to_raw().ptr as usize,
                inst: GateInst {
                    kind,
                    qubits,
                    angle,
                },
                result_keys,
                operand_keys,
            });
        }
        op = current.next_in_block();
    }

    let return_keys = return_operand_keys.unwrap_or_default();
    let return_order: Vec<u8> = return_keys
        .iter()
        .filter_map(|k| wire_to_logical.get(k).copied())
        .collect();

    let gates = if live_only {
        let mut live_values: HashSet<usize> = return_keys.into_iter().collect();
        let mut live_ops: HashSet<usize> = HashSet::new();
        for gate in recorded.iter().rev() {
            let produces_live = gate.result_keys.iter().any(|k| live_values.contains(k));
            if produces_live {
                live_ops.insert(gate.op_ptr);
                for k in &gate.operand_keys {
                    live_values.insert(*k);
                }
            }
        }
        recorded
            .into_iter()
            .filter(|g| live_ops.contains(&g.op_ptr))
            .map(|g| g.inst)
            .collect()
    } else {
        recorded.into_iter().map(|g| g.inst).collect()
    };

    Some((gates, return_order, width))
}

fn extract_region_circuit<'c, 'a>(
    op: OperationRef<'c, 'a>,
    live_only: bool,
) -> Option<CircuitSpec> {
    let region = op.region(0).ok()?;
    let block = region.first_block()?;
    let (gates, _return_order, width) = extract_from_block(block, live_only)?;
    Some(CircuitSpec::new(width, gates))
}

/// Extract a [`CircuitSpec`] from a `quantum.circ.func` (all gates in order).
pub fn extract_func_circuit<'c, 'a>(func: OperationRef<'c, 'a>) -> CircuitSpec {
    extract_region_circuit(func, false).expect("extract func circuit")
}

/// Extract the live gate cone feeding `return` from a `quantum.circ.borrow`.
pub fn extract_borrow_gates<'c, 'a>(borrow: OperationRef<'c, 'a>) -> CircuitSpec {
    extract_region_circuit(borrow, true).expect("extract borrow circuit")
}

/// Extract circuit **and** the logical indices of returned wires (SABRE).
pub fn extract_func_with_return_order<'c, 'a>(
    func: OperationRef<'c, 'a>,
) -> (CircuitSpec, Vec<u8>) {
    let region = func.region(0).expect("func region");
    let block = region.first_block().expect("func block");
    let (gates, return_order, width) = extract_from_block(block, false).expect("extract");
    (CircuitSpec::new(width, gates), return_order)
}
