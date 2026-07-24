//! Exact SMT-backed optimization for the NA pipeline (issue #302).
//!
//! Two deliverables share one z3 feature gate and the
//! timeout → heuristic-fallback pattern:
//!
//! - **Deliverable B — exact optimal placement** (OLSQ-DPQA parity):
//!   [`placement::place_exact`] solves the quadratic-assignment placement
//!   (vertex → site) as a constrained optimization in z3, minimizing the
//!   same `Σ w · √d` movement-cost score the heuristic placers compute.
//!
//! - **Deliverable A — exact state-preparation scheduling** (NASP parity):
//!   [`state_prep::schedule_exact`] encodes the CZ-pair-list scheduling
//!   problem (assign gates to movement-compatible stages) as an SMT
//!   optimization, producing the same [`crate::schedule::ScheduleLayer`]
//!   stream as the heuristic zoned path.
//!
//! Both use [`SolverOutcome`] — `Proven` (z3 found the optimum) or
//! `Timeout` (z3 exceeded its budget; the caller falls back to heuristic
//! and logs the optimality gap). The solver-free build never links z3
//! (ADR-0038 optional-z3 pattern, mirrored from `frontend`).
//!
//! [`crate::report::ScheduleOptimality`] (defined in the report module so
//! it is available without the `solver` feature) carries the schedule
//! provenance on the resource report.
//!
//! # Papers
//!
//! - **NASP** — Stade, Lin, Cong, Wille, "NASP: Neutral-Atom Scheduling and
//!   Placement", 2024 (arXiv:2409.07940).
//! - **OLSQ-DPQA** — Tan, Cong, Cong, "OLSQ-DPQA: Optimal Layout Synthesis
//!   for Diverse Quantum Processor Architectures", 2024.

pub mod placement;
pub mod state_prep;

use serde::{Deserialize, Serialize};

/// Default z3 solver timeout in milliseconds.
///
/// Short enough that a compile never stalls on the solver; long enough for
/// z3 to prove optimality on small fixtures (4–7 qubits). Callers can
/// override via [`placement::ExactPlacementParams`] /
/// [`state_prep::ExactStatePrepParams`].
pub const DEFAULT_SOLVER_TIMEOUT_MS: u64 = 5_000;

/// Whether an exact solver run produced a proven optimum or fell back.
///
/// The resource report / stats carry this so a schedule labelled `exact` is
/// genuinely solver-proven, while `heuristic` makes the fallback visible
/// (issue #302: "no silent heuristic-only fallback without logging").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolverOutcome {
    /// z3 proved the solution optimal within the time budget.
    #[default]
    Proven,
    /// z3 returned `unknown` (timeout); the caller fell back to heuristic.
    Timeout,
}

impl SolverOutcome {
    /// Convert to the report's [`crate::report::ScheduleOptimality`].
    pub fn to_schedule_optimality(self) -> crate::report::ScheduleOptimality {
        match self {
            Self::Proven => crate::report::ScheduleOptimality::Exact,
            Self::Timeout => crate::report::ScheduleOptimality::Heuristic,
        }
    }
}
