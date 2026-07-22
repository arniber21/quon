//! Clifford+T optimization pass (issue #96, ADR-0013).
//!
//! Dispatches to two MLIR-free algorithms based on the func's `clifford`
//! flag:
//!
//! * **`clifford = true`** — Aaronson–Gottesman stabilizer tableau
//!   simulation ([`stabilizer_tableau`]). Detects identity sequences
//!   (non-adjacent) and replaces them with no-ops. Also collapses
//!   single-Pauli sequences.
//!
//! * **`clifford = false`** — Phase-polynomial T-count optimization
//!   ([`phase_polynomial`]). Extracts the non-Clifford content as a sum
//!   of linear Boolean phase terms, merges/cancels non-adjacent terms
//!   algebraically, and re-synthesizes with reduced T-count.
//!
//! ## Extract / rebuild
//!
//! Each `quantum.circ.func` body is extracted to a flat gate list
//! (`Vec<(name, Vec<qubit_index>)>`), optimized by the pure-Rust kernel,
//! and rebuilt with new `quantum.circ.gate` ops if the kernel reports a
//! reduction. The func `depth` attribute is recomputed to the new gate
//! count (ADR-0013: depth may change, unlike peephole passes).
//!
//! ## Pipeline
//!
//! Circ fixpoint order: `gate_cancellation` → `rotation_merging` →
//! **`clifford_t_opt`** → `compiler_uncomputation` → `zx_simplification`
//! (ADR-0013). This pass runs after peephole cancellation so it sees
//! already-simplified IR; its non-adjacent analysis complements the
//! peephole's adjacent-only scope.

use std::collections::HashMap;

use melior::StringRef;
use melior::ir::attribute::{BoolAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{Attribute, AttributeLike, BlockLike, OperationRef, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
use mlir_sys::mlirOperationSetAttributeByName;
use quon_core::DepthExpr;

use crate::dialect::quantum_circ::{self, attr};
use crate::passes::{phase_polynomial, stabilizer_tableau};

// ---------------------------------------------------------------------------
// Helpers (same patterns as gate_cancellation / rotation_merging)
// ---------------------------------------------------------------------------

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

fn read_bool_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<bool> {
    let value = operation.attribute(key).ok()?;
    BoolAttribute::try_from(value).ok().map(|b| b.value())
}

fn read_depth_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> DepthExpr {
    read_string_attr(operation, attr::DEPTH)
        .and_then(|text| DepthExpr::parse(&text).ok())
        .unwrap_or(DepthExpr::Nat(0))
}

fn set_func_depth<'c, 'a>(context: &'c Context, func: OperationRef<'c, 'a>, depth: &DepthExpr) {
    let attribute: Attribute<'c> = StringAttribute::new(context, &depth.to_sexpr()).into();
    unsafe {
        mlirOperationSetAttributeByName(
            func.to_raw(),
            StringRef::new(attr::DEPTH).to_raw(),
            attribute.to_raw(),
        );
    }
}

fn gate_is_clifford(name: &str) -> bool {
    quon_core::gates::lookup(name).is_some_and(|g| g.class == quon_core::gates::GateClass::Clifford)
}

// ---------------------------------------------------------------------------
// Extract
// ---------------------------------------------------------------------------

/// Extract a flat gate list from a `quantum.circ.func` body block.
///
/// Returns `(gates, n_qubits)` where `gates` is `Vec<(name, Vec<qubit_index>)>`
/// with qubit indices relative to the block arguments (0-based).
/// Returns `None` if the block is not a simple linear gate chain.
#[allow(clippy::type_complexity)]
fn extract_gate_list<'c, 'a>(
    block: melior::ir::BlockRef<'c, 'a>,
) -> Option<(Vec<(String, Vec<usize>)>, usize)> {
    let n = block.argument_count();
    let mut ssa_to_wire: HashMap<usize, usize> = HashMap::new();
    for i in 0..n {
        let arg = block.argument(i).ok()?;
        ssa_to_wire.insert(value_key(&arg), i);
    }

    let mut gates = Vec::new();
    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN {
            break;
        }
        if name != quantum_circ::op::GATE {
            // Non-gate ops (compose, borrow, etc.) — not supported
            return None;
        }
        let gate_name = read_string_attr(&current, attr::GATE_NAME)?;
        let operands: Vec<Value<'c, 'a>> = current
            .operands()
            .filter(|v| quantum_circ::is_qubit_type(v.r#type()))
            .collect();
        let qubit_indices: Vec<usize> = operands
            .iter()
            .map(|v| ssa_to_wire.get(&value_key(v)).copied())
            .collect::<Option<Vec<usize>>>()?;
        // Update wire tracking with results
        let results: Vec<Value<'c, 'a>> = (0..current.result_count())
            .filter_map(|i| current.result(i).ok())
            .filter(|r| quantum_circ::is_qubit_type(r.r#type()))
            .map(Value::from)
            .collect();
        if results.len() != qubit_indices.len() {
            return None;
        }
        for (op_idx, &wire_idx) in qubit_indices.iter().enumerate() {
            ssa_to_wire.insert(value_key(&results[op_idx]), wire_idx);
        }
        gates.push((gate_name, qubit_indices));
    }
    Some((gates, n))
}

// ---------------------------------------------------------------------------
// Rebuild
// ---------------------------------------------------------------------------

/// Rebuild a func body block with a new gate list.
///
/// Inserts new `quantum.circ.gate` ops before the return, rewires the
/// return's operands, then erases all old gate ops in reverse order.
fn rebuild_block<'c, 'a>(
    context: &'c Context,
    block: melior::ir::BlockRef<'c, 'a>,
    new_gates: &[(String, Vec<usize>)],
    n_qubits: usize,
) {
    let rewriter = IrRewriter::new(context);
    let base = rewriter.as_rewriter_base();

    // Collect old gate ops (in order) and find the return op.
    let mut old_ops: Vec<OperationRef<'c, 'a>> = Vec::new();
    let mut return_op: Option<OperationRef<'c, 'a>> = None;
    let mut op = block.first_operation();
    while let Some(current) = op {
        let next = current.next_in_block();
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN {
            return_op = Some(current);
            break;
        }
        if name == quantum_circ::op::GATE {
            old_ops.push(current);
        }
        op = next;
    }
    let Some(return_op) = return_op else {
        return;
    };

    // Build new gates, inserting before the return op.
    let location = return_op.location();
    let mut wires: Vec<Value<'c, 'a>> = (0..n_qubits)
        .map(|i| Value::from(block.argument(i).expect("block argument")))
        .collect();

    for (gate_name, targets) in new_gates {
        let operands: Vec<Value<'c, 'a>> = targets.iter().map(|&i| wires[i]).collect();
        let is_clifford = gate_is_clifford(gate_name);
        let new_op = block.insert_operation_before(
            return_op,
            quantum_circ::gate(context, gate_name, 1, is_clifford, &operands, location)
                .expect("rebuilt gate builds"),
        );
        for (i, &target) in targets.iter().enumerate() {
            wires[target] = Value::from(new_op.result(i).expect("rebuilt result"));
        }
    }

    // Rewire return: replace old qubit operands with new final wires.
    let return_qubit_vals: Vec<Value<'c, 'a>> = return_op
        .operands()
        .filter(|v| quantum_circ::is_qubit_type(v.r#type()))
        .collect();
    for (i, old_val) in return_qubit_vals.iter().enumerate() {
        if i < wires.len() {
            base.replace_all_uses_with(*old_val, wires[i]);
        }
    }

    // Erase old gate ops in reverse order (last gate first).
    for op in old_ops.into_iter().rev() {
        base.erase_op(op);
    }
}

// ---------------------------------------------------------------------------
// Per-func optimization
// ---------------------------------------------------------------------------

/// Optimize a single `quantum.circ.func` op in-place.
fn optimize_func<'c, 'a>(context: &'c Context, func: OperationRef<'c, 'a>) {
    if op_name(&func) != quantum_circ::op::FUNC {
        return;
    }
    let Ok(region) = func.region(0) else {
        return;
    };
    let Some(block) = region.first_block() else {
        return;
    };

    let func_clifford = read_bool_attr(&func, attr::CLIFFORD).unwrap_or(false);
    let Some((gate_list, n_qubits)) = extract_gate_list(block) else {
        return; // not a simple gate chain
    };
    if gate_list.is_empty() {
        return;
    }

    let optimized = if func_clifford {
        // Stabilizer tableau path: only if all gates are tableau-supported.
        if !stabilizer_tableau::is_all_tableau(&gate_list) {
            return;
        }
        stabilizer_tableau::optimize_clifford(&gate_list, n_qubits)
    } else {
        // Phase polynomial path: optimize T-count on CNOT+T blocks.
        phase_polynomial::optimize_t_count(&gate_list, n_qubits)
    };

    let Some(new_gates) = optimized else {
        return; // no improvement
    };

    // Rebuild the block with the optimized gate list.
    rebuild_block(context, block, &new_gates, n_qubits);

    // Recompute depth (ADR-0013: depth may change).
    if let DepthExpr::Nat(_) = read_depth_attr(&func) {
        let new_depth = DepthExpr::Nat(new_gates.len() as u64);
        set_func_depth(context, func, &new_depth);
    }
}

/// Optimize every `quantum.circ.func` in a module.
fn optimize_module<'c, 'a>(context: &'c Context, module: OperationRef<'c, 'a>) {
    let Some(body) = module
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return;
    };
    let mut op = body.first_operation();
    while let Some(current) = op {
        if op_name(&current) == quantum_circ::op::FUNC {
            optimize_func(context, current);
        }
        op = current.next_in_block();
    }
}

/// Runs Clifford+T optimization on every `quantum.circ.func` in `module`.
pub fn run_on_module<'c>(context: &'c Context, module: &melior::ir::Module<'c>) {
    optimize_module(context, module.as_operation());
}

// ---------------------------------------------------------------------------
// External pass registration
// ---------------------------------------------------------------------------

#[repr(align(8))]
struct PassId;

static CLIFFORD_T_OPT_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct CliffordTOpt {
    context: usize,
}

impl CliffordTOpt {
    fn new() -> Self {
        Self { context: 0 }
    }
}

impl<'c> RunExternalPass<'c> for CliffordTOpt {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        if self.context == 0 {
            pass.signal_failure();
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        optimize_module(context, operation);
    }
}

/// Creates the Clifford+T optimization pass.
pub fn create_pass() -> Pass {
    create_external(
        CliffordTOpt::new(),
        TypeId::create(&CLIFFORD_T_OPT_PASS_ID),
        "clifford-t-opt",
        "clifford-t-opt",
        "Phase-polynomial T-count + stabilizer-tableau Clifford optimization",
        "",
        &[],
    )
}
