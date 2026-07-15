//! Shared compile-pipeline stages for Fixed (OpenQASM) targets.
//!
//! # Stages
//!
//! Callable without `quonc`. Order matches the implemented SPEC §7 Fixed path
//! (see also [`crate::passes`] module docs):
//!
//! 1. **Circ fixpoint** ([`run_circ_passes_to_fixpoint`]) — `gate_cancellation`,
//!    `rotation_merging`, `compiler_uncomputation`, `zx_simplification` to
//!    fixpoint. `clifford_t_opt` is reserved for #96 and is **not** run (#214).
//! 2. **Monadic lowering** — call [`crate::passes::monadic_lowering::run_on_module`]
//!    (circ → dynamic).
//! 3. **Dynamic passes** ([`run_dynamic_passes`]) — `measurement_deferral`,
//!    `classical_region_fusion`.
//! 4. **Fixed physical** ([`run_fixed_physical`]) — strict order:
//!    `native_gate_decomp` → `sabre_routing` → `native_gate_decomp` (post-SWAP)
//!    → `depth_scheduling`. T-count is sampled after SABRE and before the
//!    second decomp (same hook as the historical `quonc` driver).
//! 5. **OpenQASM emit** ([`emit_openqasm`]) — orchestration hook over
//!    [`crate::emit::openqasm3`].
//!
//! Neutral-atom scheduling after dynamic IR lives in `quon_na::pipeline`.
//!
//! QEC: after monadic lowering, [`crate::collect_qec_workload`] builds MLIR-free
//! workload IR (#251). Calling it from this pipeline / expanding to `quantum.na`
//! is issue #248.

use backend::BackendTarget;
use melior::Context;
use melior::ir::Module;

use crate::emit::openqasm3;
use crate::metrics;
use crate::passes::{
    classical_region_fusion, compiler_uncomputation, depth_scheduling, gate_cancellation,
    measurement_deferral, native_gate_decomp, rotation_merging,
    sabre_routing::{self, SabreCost},
    zx_simplification,
};

/// Result of the Fixed physical pass sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixedPhysicalResult {
    /// T-count sampled after SABRE and before the post-SWAP `native_gate_decomp`.
    pub t_count: u64,
}

/// Runs `quantum.circ` optimization passes to fixpoint (SPEC §7.1 passes 1–4).
///
/// `clifford_t_opt` is intentionally absent: #214 removed the shallow alias that
/// only re-ran `gate_cancellation`. Real Clifford+T is #96.
pub fn run_circ_passes_to_fixpoint(context: &Context, module: &Module<'_>) {
    const MAX_ROUNDS: usize = 10;
    for _ in 0..MAX_ROUNDS {
        let before = module.as_operation().to_string();
        gate_cancellation::run_on_module(context, module);
        rotation_merging::run_on_module(context, module);
        compiler_uncomputation::run_on_module(context, module);
        zx_simplification::run_on_module(context, module);
        let after = module.as_operation().to_string();
        if before == after {
            break;
        }
    }
}

/// Runs `quantum.dynamic` passes after monadic lowering (SPEC §7.1 passes 6–7).
pub fn run_dynamic_passes(context: &Context, module: &Module<'_>) {
    measurement_deferral::run_on_module(context, module);
    classical_region_fusion::run_on_module(context, module);
}

/// Runs Fixed physical passes in the implemented strict order.
///
/// Order: `native_gate_decomp` → `sabre_routing` → `native_gate_decomp` →
/// `depth_scheduling`. Returns the pre–post-SWAP T-count for metrics.
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

/// OpenQASM 3.0 emission hook used by the Fixed compile path.
pub fn emit_openqasm(
    module: &Module<'_>,
    target: &BackendTarget,
) -> Result<String, openqasm3::EmitError> {
    openqasm3::emit(module, target)
}

/// Optional stderr IR dump between pipeline stages (preserves `quonc --dump-ir`).
pub fn dump_ir_stage(enabled: bool, label: &str, module: &Module<'_>) {
    if enabled {
        eprintln!("--- {label} ---\n{}", module.as_operation());
    }
}
