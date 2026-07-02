// Pass registration — all passes are registered as Melior external passes.
// Pipeline order (SPEC.md §7.1):
//
//   quantum.circ passes (run to fixpoint before lowering to quantum.dynamic):
//     1. gate_cancellation
//     2. rotation_merging
//     3. compiler_uncomputation
//     4. zx_simplification
//     5. clifford_t_opt
//
//   quantum.dynamic passes:
//     6. measurement_deferral
//     7. classical_region_fusion
//
//   quantum.physical passes (after physical lowering, strict order):
//     8. native_gate_decomp
//     9. sabre_routing
//    10. depth_scheduling

pub mod classical_region_fusion;
pub mod clifford_t_opt;
pub mod compiler_uncomputation;
pub mod depth_scheduling;
pub mod dynamic_linearity_verifier;
pub mod gate_cancellation;
pub mod linearity_verifier;
pub mod measurement_deferral;
pub mod monadic_lowering;
pub mod native_gate_decomp;
pub(crate) mod qubit_wiring;
pub mod rotation_merging;
pub mod sabre_routing;
pub mod zx_simplification;
