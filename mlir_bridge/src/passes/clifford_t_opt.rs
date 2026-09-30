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
//! Each `quantum.circ.func` body is extracted through [`circ_extract::extract`]
//! (the shared seam). The resulting [`circ_extract::CircIr`] is mapped to the
//! flat `(canonical name, qubit indices)` list the kernels consume. Names come
//! from the gate registry, so aliases such as `CX` are `CNOT` before the
//! tableau and phase-polynomial predicates run. The optimized list is rebuilt
//! with new `quantum.circ.gate` ops when the kernel reports a reduction. The
//! func `depth` attribute is recomputed to the new gate count (ADR-0013: depth
//! may change, unlike peephole passes). Extraction errors decline the rewrite.
//!
//! ## Pipeline
//!
//! Circ fixpoint order: `gate_cancellation` → `rotation_merging` →
//! **`clifford_t_opt`** → `compiler_uncomputation` → `zx_simplification`
//! (ADR-0013). This pass runs after peephole cancellation so it sees
//! already-simplified IR; its non-adjacent analysis complements the
//! peephole's adjacent-only scope.

use melior::ir::attribute::{BoolAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{Attribute, BlockLike, OperationRef, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
use quon_core::DepthExpr;
use quon_core::gates::GateId;

use crate::circ_extract::{self, SeamError};
use crate::dialect::quantum_circ::{self, attr};
use crate::ffi::{self, PassContext};
use crate::passes::{phase_polynomial, stabilizer_tableau};

// ---------------------------------------------------------------------------
// Helpers (same patterns as gate_cancellation / rotation_merging)
// ---------------------------------------------------------------------------

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
    ffi::set_operation_attribute(func, attr::DEPTH, &attribute);
}

fn gate_is_clifford(id: GateId) -> bool {
    id.info().class == quon_core::gates::GateClass::Clifford
}

// ---------------------------------------------------------------------------
// Extract
// ---------------------------------------------------------------------------

/// Map extracted [`circ_extract::CircIr`] into the kernel gate list.
///
/// Gate names are registry ids (`quon_core::gates`), not the raw attribute
/// text, so `CX` and `CNOT` are the same predicate input.
fn gate_list(circ: &circ_extract::CircIr) -> Vec<(GateId, Vec<usize>)> {
    circ.gates
        .iter()
        .map(|gate| (gate.name, gate.qubits.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Rebuild
// ---------------------------------------------------------------------------

/// Rebuild a func body block with a new gate list.
///
/// Inserts new `quantum.circ.gate` ops before the return, rewires the
/// return's operands, then erases all old gate ops in reverse order.
///
/// Returns `Err` without leaving a half-built body when a wire index, block
/// argument, or gate builder is invalid. The caller declines the rewrite.
fn rebuild_block<'c, 'a>(
    context: &'c Context,
    block: melior::ir::BlockRef<'c, 'a>,
    new_gates: &[(GateId, Vec<usize>)],
    n_qubits: usize,
) -> Result<(), SeamError> {
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
        return Err(SeamError::NoTerminator);
    };

    for (_, targets) in new_gates {
        for &index in targets {
            if index >= n_qubits {
                return Err(SeamError::WireOutOfRange { index, n_qubits });
            }
        }
    }

    // Build new gates, inserting before the return op.
    let location = return_op.location();
    let mut wires: Vec<Value<'c, 'a>> = Vec::with_capacity(n_qubits);
    for index in 0..n_qubits {
        let argument = block
            .argument(index)
            .map_err(|error| SeamError::Build(error.to_string()))?;
        wires.push(Value::from(argument));
    }

    let mut inserted: Vec<OperationRef<'c, 'a>> = Vec::new();
    for (gate_name, targets) in new_gates {
        let operands: Vec<Value<'c, 'a>> = targets.iter().map(|&i| wires[i]).collect();
        let is_clifford = gate_is_clifford(*gate_name);
        let built = quantum_circ::gate(
            context,
            gate_name.as_str(),
            1,
            is_clifford,
            &operands,
            location,
        )
        .map_err(|error| SeamError::Build(error.to_string()));
        let built = match built {
            Ok(operation) => operation,
            Err(error) => {
                erase_inserted(&base, &inserted);
                return Err(error);
            }
        };
        let new_op = block.insert_operation_before(return_op, built);
        inserted.push(new_op);
        for (i, &target) in targets.iter().enumerate() {
            match new_op.result(i) {
                Ok(result) => wires[target] = Value::from(result),
                Err(error) => {
                    erase_inserted(&base, &inserted);
                    return Err(SeamError::Build(error.to_string()));
                }
            }
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
    Ok(())
}

fn erase_inserted<'c, 'a>(base: &melior::RewriterBase<'c, '_>, inserted: &[OperationRef<'c, 'a>]) {
    for op in inserted.iter().rev() {
        base.erase_op(*op);
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
    // Shared seam (#320). Decline on structural ops, unknown gates, or arity
    // mismatches instead of walking SSA wires a second time.
    let circ = match circ_extract::extract(func) {
        Ok(circ) => circ,
        Err(_) => return,
    };
    let n_qubits = circ.n_qubits;
    let gate_list = gate_list(&circ);
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

    // Rebuild the block with the optimized gate list. A builder failure
    // declines the rewrite and leaves the original body in place.
    if let Err(error) = rebuild_block(context, block, &new_gates, n_qubits) {
        eprintln!("warning: clifford_t_opt declined a rewrite: {error}");
        return;
    }

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
    context: PassContext,
}

impl CliffordTOpt {
    fn new() -> Self {
        Self {
            context: PassContext::new(),
        }
    }
}

impl<'c> RunExternalPass<'c> for CliffordTOpt {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context.capture(context);
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        let Some(raw) = self.context.raw() else {
            pass.signal_failure();
            return;
        };
        crate::ffi::with_context(raw, |context| {
            optimize_module(context, operation);
        });
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

#[cfg(test)]
mod tests {
    use super::{GateId, rebuild_block};
    use crate::circ_extract::SeamError;
    use crate::dialect::quantum_circ as qc;

    use melior::ir::operation::OperationLike;
    use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike};

    #[test]
    fn rebuild_declines_out_of_range_wire_without_rewriting() {
        let context = melior::Context::new();
        qc::register_dialect(&context);
        let location = Location::unknown(&context);
        let qubit = qc::qubit_type(&context);
        let block = Block::new(&[(qubit, location)]);
        let wire = melior::ir::Value::from(block.argument(0).expect("arg"));
        let gate = block
            .append_operation(qc::gate(&context, "H", 1, true, &[wire], location).expect("gate"));
        let out = melior::ir::Value::from(gate.result(0).expect("result"));
        block.append_operation(qc::r#return(&[out], location).expect("return"));
        let region = Region::new();
        region.append_block(block);
        let func = qc::func(
            &context,
            "main",
            1,
            1,
            &quon_core::DepthExpr::Nat(1),
            true,
            region,
            location,
        )
        .expect("func");
        let module = Module::new(location);
        module.body().append_operation(func);
        let before = module.as_operation().to_string();

        let body = module
            .body()
            .first_operation()
            .expect("func")
            .region(0)
            .expect("region")
            .first_block()
            .expect("block");
        let error = rebuild_block(
            &context,
            body,
            &[(GateId::parse("X").expect("X"), vec![5])],
            1,
        )
        .expect_err("out-of-range wire");

        assert_eq!(
            error,
            SeamError::WireOutOfRange {
                index: 5,
                n_qubits: 1
            }
        );
        assert_eq!(
            module.as_operation().to_string(),
            before,
            "a declined rebuild must leave the original gates in place"
        );
    }
}
