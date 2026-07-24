//! ZX simplification pass (issue #20).
//!
//! Uses the shared `circ_extract` seam (issue #320) for faithful extract/rebuild.
//! The ZX kernel (`zx` crate) owns the graph algebra; this pass owns only the
//! Melior ↔ CircIr adaptation and the ZX-encodability gate.

use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{BlockLike, OperationRef, RegionLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef};
use zx::{GateRef, circuit_to_zx, simplify, zx_to_circuit};

use crate::circ_extract;
use crate::dialect::{quantum_circ, quantum_dynamic};

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// Extracts gate metadata from a `quantum.circ.func` for ZX translation.
///
/// Thin wrapper over the shared [`circ_extract::extract`] seam. Returns an
/// empty vec when extraction declines (unsupported construct, unknown gate),
/// so callers that check `is_empty()` treat it as "no rewrite".
pub fn extract_gates<'c, 'a>(func: OperationRef<'c, 'a>) -> Vec<GateRef> {
    circ_extract::extract(func)
        .map(|circ| {
            circ.gates
                .iter()
                .map(circ_extract::circ_gate_to_gate_ref)
                .collect()
        })
        .unwrap_or_default()
}

/// Returns true when ZX rewriting shrinks the gate list for `func`.
///
/// Uses the shared [`circ_extract`] seam (issue #320) for faithful extract/
/// rebuild. Still restricted to single-wire funcs: the ZX kernel
/// ([`zx_to_circuit`]) only round-trips independent single-qubit chains, so
/// multi-qubit funcs are declined. Multi-qubit ZX extraction is a follow-up
/// to #75.
pub fn simplify_func<'c, 'a>(context: &'c Context, func: OperationRef<'c, 'a>) -> bool {
    // The ZX kernel only round-trips independent single-qubit chains.
    if func_qubit_count(func) != 1 {
        return false;
    }
    // Faithful extraction through the shared seam (issue #320).
    let circ = match circ_extract::extract(func) {
        Ok(circ) => circ,
        Err(_) => return false, // unsupported construct — decline
    };
    if circ.gates.len() < 2 {
        return false;
    }
    // Gate names are canonical (from the registry); check ZX-encodability.
    if circ.gates.iter().any(|gate| !zx_encodable(&gate.name)) {
        return false;
    }
    // Convert to ZX kernel format and simplify.
    let gates: Vec<GateRef> = circ
        .gates
        .iter()
        .map(circ_extract::circ_gate_to_gate_ref)
        .collect();
    let mut zx = circuit_to_zx(&gates);
    let before = gates.len();
    simplify(&mut zx);
    let simplified = zx_to_circuit(&zx);
    // Empty means the ZX extractor declined (H edges / branching); never
    // treat as identity.
    if simplified.is_empty() || simplified.len() >= before {
        return false;
    }
    // Convert back to CircIr and rebuild through the shared seam.
    let new_circ = circ_extract::CircIr {
        n_qubits: circ.n_qubits,
        gates: simplified
            .iter()
            .map(circ_extract::gate_ref_to_circ_gate)
            .collect(),
    };
    circ_extract::rebuild(context, func, &new_circ).is_ok()
}

/// Gates the ZX encode/extract path can handle without silently dropping ops.
///
/// Names are canonical (from the gate registry), so only canonical ids appear.
/// Unsupported names (S, T, Y, SWAP, …) are ignored by `circuit_to_zx`, so a
/// rewrite that only checked `simplified.len() < before` could delete them.
fn zx_encodable(name: &str) -> bool {
    matches!(name, "H" | "X" | "Z" | "Rx" | "Rz" | "CNOT")
}

/// Number of qubit arguments on `func`'s entry block (0 if it has no body).
fn func_qubit_count<'c, 'a>(func: OperationRef<'c, 'a>) -> usize {
    func.region(0)
        .ok()
        .and_then(|region| region.first_block())
        .map(|block| block.argument_count())
        .unwrap_or(0)
}

fn simplify_module<'c, 'a>(context: &'c Context, module: OperationRef<'c, 'a>) {
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
        let name = op_name(&current);
        if name == quantum_circ::op::FUNC || name == quantum_dynamic::op::UNITARY_REGION {
            // `simplify_func` operates on any op whose region(0) is a
            // circ-only body terminated by `quantum.circ.return` — after the
            // staging-dialect collapse (#213 / ADR-0037) that includes
            // `quantum.dynamic.unitary_region` bodies, not just `func` defs.
            simplify_func(context, current);
        }
    }
}

/// Runs ZX simplification on `module`.
pub fn run_on_module<'c>(context: &'c Context, module: &melior::ir::Module<'c>) {
    simplify_module(context, module.as_operation());
}

#[repr(align(8))]
struct PassId;

static ZX_SIMPLIFICATION_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct ZxSimplification {
    context: usize,
}

impl ZxSimplification {
    fn new() -> Self {
        Self { context: 0 }
    }
}

impl<'c> RunExternalPass<'c> for ZxSimplification {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, _pass: ExternalPass<'_>) {
        if self.context == 0 {
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        simplify_module(context, operation);
    }
}

/// Creates the ZX simplification pass.
pub fn create_pass() -> Pass {
    create_external(
        ZxSimplification::new(),
        TypeId::create(&ZX_SIMPLIFICATION_PASS_ID),
        "zx-simplification",
        "zx-simplification",
        "Simplify quantum.circ.func regions via the zx crate",
        "",
        &[],
    )
}
