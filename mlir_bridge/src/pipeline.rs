//! Shared compile-pipeline stages for Fixed (OpenQASM) targets.
//!
//! # Stages
//!
//! Callable without `quonc`. Order matches the implemented SPEC §7 Fixed path
//! (see also [`crate::passes`] module docs):
//!
//! 1. **Circ fixpoint** ([`run_circ_passes_to_fixpoint`]) — `gate_cancellation`,
//!    `rotation_merging`, `clifford_t_opt`, `compiler_uncomputation`,
//!    `zx_simplification` to fixpoint (ADR-0013 / #96).
//! 2. **Dynamic passes** ([`run_dynamic_passes`]) — `measurement_deferral`,
//!    `classical_region_fusion`. (`frontend::lower` already emits
//!    `quantum.dynamic` IR directly — the staging dialect / lowering pass was
//!    collapsed in #213 / ADR-0037.)
//! 3. **Fixed physical** ([`run_fixed_physical`] in [`crate::fixed_physical`]) —
//!    strict order: `native_gate_decomp` → `sabre_routing` →
//!    `native_gate_decomp` (post-SWAP) → `depth_scheduling`. T-count is sampled
//!    after SABRE and before the second decomp (same hook as the historical
//!    `quonc` driver). SSA wiring is the canonical layout channel; `phys_qubit`
//!    is a derived annotation (ADR-0034).
//! 4. **OpenQASM emit** ([`emit_openqasm`]) — orchestration hook over
//!    [`crate::emit::openqasm3`].
//!
//! Neutral-atom scheduling after dynamic IR lives in `quon_na::pipeline`.
//!
//! QEC: after lowering, [`crate::collect_qec_workload`] builds MLIR-free
//! workload IR (#251). Calling it from this pipeline / expanding to `quantum.na`
//! is issue #248.

use backend::BackendTarget;
use melior::Context;
use melior::ir::Module;

use crate::emit::openqasm3;
use crate::passes::{
    classical_region_fusion, clifford_t_opt, compiler_uncomputation, depth_scheduling,
    gate_cancellation, measurement_deferral, native_gate_decomp, rotation_merging,
    sabre_routing::{self, SabreCost},
    zx_simplification,
};

// The Fixed physical pipeline (decomp → route → decomp → schedule) and its
// result type live in [`crate::fixed_physical`] — the one deep module owning
// Fixed physical layout (ADR-0034). Re-exported here for call-site stability.
pub use crate::fixed_physical::{FixedPhysicalResult, run_fixed_physical};

/// Runs `quantum.circ` optimization passes to fixpoint (SPEC §7.1, ADR-0013).
///
/// Fixpoint order: `gate_cancellation` → `rotation_merging` → `clifford_t_opt`
/// → `compiler_uncomputation` → `zx_simplification`. The `clifford_t_opt` pass
/// performs non-adjacent Clifford simplification (stabilizer tableau) and
/// T-count reduction (phase polynomial) — see ADR-0013 / #96.
pub fn run_circ_passes_to_fixpoint(context: &Context, module: &Module<'_>) {
    const MAX_ROUNDS: usize = 10;
    for _ in 0..MAX_ROUNDS {
        let before = module.as_operation().to_string();
        gate_cancellation::run_on_module(context, module);
        rotation_merging::run_on_module(context, module);
        clifford_t_opt::run_on_module(context, module);
        compiler_uncomputation::run_on_module(context, module);
        zx_simplification::run_on_module(context, module);
        let after = module.as_operation().to_string();
        if before == after {
            break;
        }
    }
}

/// Runs `quantum.dynamic` passes after lowering (SPEC §7.1 passes 6–7).
pub fn run_dynamic_passes(context: &Context, module: &Module<'_>) {
    measurement_deferral::run_on_module(context, module);
    classical_region_fusion::run_on_module(context, module);
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
