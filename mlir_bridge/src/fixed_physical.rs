//! Fixed physical layout module (issue #316, ADR-0034).
//!
//! One deep module owning the `TargetKind::Fixed` physical pipeline:
//! `native_gate_decomp` → `sabre_routing` → `native_gate_decomp` (post-SWAP)
//! → `depth_scheduling`. Emit, T-count sampling, and metrics consume this
//! module's result rather than re-deriving layout from a second channel.
//!
//! # Canonical channel: SSA wiring
//!
//! Fixed physical layout identity has **one authoritative representation**:
//! the SSA qubit wiring that SABRE produces by rewriting operands and inserting
//! SWAP gates. Both OpenQASM emit ([`crate::emit::openqasm3`]) and depth
//! scheduling ([`crate::passes::depth_scheduling`]) derive qubit identity from
//! this SSA wiring — emit via its own value→register threading, scheduling via
//! [`crate::dynamic_walk::resolve_phys_qubits`] (which returns WireTracker
//! roots, the SSA-identity channel). Because both consume the same channel,
//! they cannot disagree on which physical qubit a gate touches.
//!
//! The `phys_qubit` attribute is a **derived annotation**: SABRE writes it from
//! its `Layout` (itself a consequence of SSA rewiring) as a per-gate snapshot
//! of the first operand's physical index. It is consumed only by metrics for
//! `phys_qubit` attr after routing leaves emit and scheduling unchanged,
//! because they read SSA, not the attr (see `corrupt_phys_qubit_attrs` and
//! the tests in `tests/fixed_physical.rs`).
//!
//! Neutral-atom scheduling stays on `quantum.na` / `ScheduleLayer` →
//! `ScheduleSpec` (ADR-0007, ADR-0011, ADR-0009) and is NOT part of this
//! module.

use backend::BackendTarget;
use melior::Context;
use melior::StringRef;
use melior::ir::Module;
use melior::ir::attribute::IntegerAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::r#type::IntegerType;
use melior::ir::{AttributeLike, BlockLike, OperationRef, RegionLike};
use mlir_sys::mlirOperationSetAttributeByName;

use crate::dialect::quantum_dynamic;
use crate::metrics;
use crate::passes::{
    depth_scheduling, native_gate_decomp, sabre_routing, sabre_routing::SabreCost,
};

/// Result of the Fixed physical pass sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixedPhysicalResult {
    /// T-count sampled after SABRE and before the post-SWAP `native_gate_decomp`.
    pub t_count: u64,
}

/// Runs Fixed physical passes in the implemented strict order.
///
/// Order: `native_gate_decomp` → `sabre_routing` → `native_gate_decomp` →
/// `depth_scheduling`. Returns the pre–post-SWAP T-count for metrics.
///
/// SABRE rewrites SSA operands (the canonical layout channel) and writes
/// `phys_qubit` attrs as a derived annotation. Scheduling and emit both
/// consume the SSA wiring — see the module docs.
pub fn run_fixed_physical(
    context: &Context,
    target: &BackendTarget,
    sabre_cost: SabreCost,
    module: &Module<'_>,
) -> FixedPhysicalResult {
    native_gate_decomp::run_on_module(context, target, module);
    sabre_routing::run_on_module(context, target, sabre_cost, module);
    let t_count = metrics::count_t_gates(module);
    native_gate_decomp::run_on_module(context, target, module);
    depth_scheduling::run_on_module(context, target, module);
    FixedPhysicalResult { t_count }
}

// ─── Derived-annotation helpers ─────────────────────────────────────────────

/// Overwrites every `phys_qubit` attribute on every reachable op in `module`
/// with `bogus`, walking the top-level block, nested `unitary_region`/`if`
/// bodies, and named `quantum.circ.func` bodies.
///
/// This is the **deletion test** for the canonical-channel decision (ADR-0034):
/// because emit and scheduling derive qubit identity from SSA wiring, not from
/// the `phys_qubit` attr, corrupting every attr after routing leaves their
/// output unchanged. Used by the integration tests in `tests/fixed_physical.rs`.
pub fn corrupt_phys_qubit_attrs(context: &Context, module: &Module<'_>, bogus: i32) {
    let attr: melior::ir::Attribute<'_> =
        IntegerAttribute::new(IntegerType::new(context, 32).into(), i64::from(bogus)).into();
    let raw = attr.to_raw();
    let name = StringRef::new(quantum_dynamic::attr::PHYS_QUBIT).to_raw();
    let Some(body) = module
        .as_operation()
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return;
    };
    corrupt_block(body, name, raw);
}

fn corrupt_block<'c, 'a>(
    block: melior::ir::BlockRef<'c, 'a>,
    name: mlir_sys::MlirStringRef,
    attr: mlir_sys::MlirAttribute,
) {
    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        corrupt_op(current, name, attr);
    }
}

fn corrupt_op<'c, 'a>(
    op: OperationRef<'c, 'a>,
    name: mlir_sys::MlirStringRef,
    attr: mlir_sys::MlirAttribute,
) {
    // Only ops that already have a phys_qubit attr are touched — overwriting
    // a non-existent attr would *add* one, which is not the test's intent.
    if op.attribute(quantum_dynamic::attr::PHYS_QUBIT).is_ok() {
        unsafe {
            mlirOperationSetAttributeByName(op.to_raw(), name, attr);
        }
    }
    let count = op.region_count();
    for index in 0..count {
        if let Ok(region) = op.region(index)
            && let Some(inner) = region.first_block()
        {
            corrupt_block(inner, name, attr);
        }
    }
}
