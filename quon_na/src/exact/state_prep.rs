//! Exact state-preparation scheduling via z3 SMT (issue #302, Deliverable A).
//!
//! Formulated after NASP (Stade et al. 2024): given a list of CZ entangling
//! pairs, schedule them into the minimum number of movement-compatible
//! stages — the same [`ScheduleLayer`] stream the heuristic zoned path
//! ([`crate::zoned::schedule_zoned`]) produces.
//!
//! # Encoding
//!
//! This is a graph-colouring problem: each gate is a vertex, two gates are
//! adjacent (cannot share a stage) iff they share an atom. The minimum
//! stage count is the chromatic number of the conflict graph.
//!
//! We solve it with z3 via binary search on the stage count `k`: for each
//! `k`, we create integer variables `stage[g] ∈ [0, k)` and assert that
//! adjacent gates have different stages. If satisfiable, we try a smaller
//! `k`; otherwise, we try larger. This finds the exact optimum.
//!
//! # Timeout / fallback
//!
//! [`ExactStatePrepParams::timeout_ms`] bounds each z3 call. On `unknown`
//! the caller falls back to the heuristic zoned scheduler and logs the gap.

use z3::ast::{Ast, Int};
use z3::{Config, Context, SatResult, Solver};

use crate::exact::{DEFAULT_SOLVER_TIMEOUT_MS, SolverOutcome};
use crate::layout::AtomId;
use crate::schedule::{NeutralAtomAction, ScheduleLayer};

/// Tunable parameters for [`schedule_exact`].
#[derive(Clone, Copy, Debug)]
pub struct ExactStatePrepParams {
    /// z3 solver timeout (milliseconds) per SAT check.
    pub timeout_ms: u64,
}

impl Default for ExactStatePrepParams {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_SOLVER_TIMEOUT_MS,
        }
    }
}

/// Result of exact state-prep scheduling.
pub struct ExactStatePrepResult {
    /// Scheduled layers: one entangle layer per stage.
    pub layers: Vec<ScheduleLayer>,
    /// Number of distinct stages (movement rounds).
    pub stage_count: usize,
    pub outcome: SolverOutcome,
}

/// One CZ gate: the two atoms it entangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CzGate {
    pub a: AtomId,
    pub b: AtomId,
}

/// Schedule a list of CZ gates into the minimum number of stages using z3.
///
/// Two gates are incompatible (cannot share a stage) if they share an atom.
/// The solver finds the minimum stage count via binary search on `k`.
pub fn schedule_exact(
    gates: &[CzGate],
    params: ExactStatePrepParams,
) -> Result<ExactStatePrepResult, String> {
    let n = gates.len();
    if n == 0 {
        return Ok(ExactStatePrepResult {
            layers: Vec::new(),
            stage_count: 0,
            outcome: SolverOutcome::Proven,
        });
    }

    // Build adjacency: pairs of gate indices that share an atom.
    let mut adj: Vec<(usize, usize)> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            let shares = gates[i].a == gates[j].a
                || gates[i].a == gates[j].b
                || gates[i].b == gates[j].a
                || gates[i].b == gates[j].b;
            if shares {
                adj.push((i, j));
            }
        }
    }

    // Lower bound: max degree + 1 (Brooks' theorem gives Δ+1 as an upper
    // bound too, but for a lower bound we use the clique number / Δ).
    let mut degree = vec![0usize; n];
    for &(i, j) in &adj {
        degree[i] += 1;
        degree[j] += 1;
    }
    // Start from max_degree (an upper bound by Brooks' theorem, except for
    // complete graphs / odd cycles where it's Δ+1) and search downward.
    // If the starting k is UNSAT, search upward.
    let start_k = degree.iter().copied().max().unwrap_or(0).max(1);
    let ub = n;

    let mut best_assignment: Option<Vec<i64>> = None;
    let mut timed_out = false;

    let mut k = start_k;
    while k >= 1 && k <= ub {
        match try_k_colouring(gates, &adj, k, params.timeout_ms) {
            KColourResult::Sat(assignment) => {
                best_assignment = Some(assignment);
                if k == 1 {
                    break;
                }
                k -= 1;
            }
            KColourResult::Unsat => {
                if best_assignment.is_some() {
                    break; // found the minimum: last SAT k
                }
                // start_k was too low (complete graph / odd cycle); go up.
                k += 1;
            }
            KColourResult::Unknown => {
                timed_out = true;
                if best_assignment.is_none() {
                    best_assignment = Some(greedy_colouring(gates, &adj));
                }
                break;
            }
        }
    }

    let outcome = if timed_out {
        SolverOutcome::Timeout
    } else {
        SolverOutcome::Proven
    };

    let stages = best_assignment.ok_or_else(|| "no colouring found".to_string())?;
    let max_stage = stages.iter().copied().max().unwrap_or(0);
    let stage_count = (max_stage + 1) as usize;

    let mut layers = Vec::with_capacity(stage_count);
    for s in 0..stage_count {
        let mut actions = Vec::new();
        for (i, &stage) in stages.iter().enumerate() {
            if stage as usize == s {
                actions.push(NeutralAtomAction::Entangle2 {
                    atoms: [gates[i].a, gates[i].b],
                    duration_us: 1,
                });
            }
        }
        if !actions.is_empty() {
            layers.push(ScheduleLayer {
                cycle: s as u32,
                actions,
            });
        }
    }

    Ok(ExactStatePrepResult {
        layers,
        stage_count,
        outcome,
    })
}

enum KColourResult {
    Sat(Vec<i64>),
    Unsat,
    Unknown,
}

fn try_k_colouring(
    gates: &[CzGate],
    adj: &[(usize, usize)],
    k: usize,
    timeout_ms: u64,
) -> KColourResult {
    let n = gates.len();
    let k_i64 = k as i64;

    let mut cfg = Config::new();
    cfg.set_timeout_msec(timeout_ms);
    let ctx = Context::new(&cfg);
    let solver = Solver::new(&ctx);

    let stage_vars: Vec<Int> = (0..n)
        .map(|i| Int::new_const(&ctx, format!("st{i}")))
        .collect();

    for sv in &stage_vars {
        solver.assert(&sv.ge(&Int::from_i64(&ctx, 0)));
        solver.assert(&sv.lt(&Int::from_i64(&ctx, k_i64)));
    }

    for &(i, j) in adj {
        solver.assert(&stage_vars[i]._eq(&stage_vars[j]).not());
    }

    match solver.check() {
        SatResult::Sat => match solver.get_model() {
            // `get_model` can return `None` even after `Sat` (model
            // extraction failure). Treat that as a timeout rather than
            // panicking, so the caller falls back to the greedy colouring
            // (issue #302: graceful degradation, no crash).
            None => KColourResult::Unknown,
            Some(model) => {
                let stages: Vec<i64> = stage_vars
                    .iter()
                    .map(|sv| model.eval(sv, true).and_then(|v| v.as_i64()).unwrap_or(0))
                    .collect();
                KColourResult::Sat(stages)
            }
        },
        SatResult::Unsat => KColourResult::Unsat,
        SatResult::Unknown => KColourResult::Unknown,
    }
}

/// Greedy graph colouring fallback (used on z3 timeout).
fn greedy_colouring(gates: &[CzGate], adj: &[(usize, usize)]) -> Vec<i64> {
    let n = gates.len();
    let mut colors = vec![-1i64; n];

    let mut adj_list: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(i, j) in adj {
        adj_list[i].push(j);
        adj_list[j].push(i);
    }

    for v in 0..n {
        let mut used: std::collections::BTreeSet<i64> = std::collections::BTreeSet::new();
        for &u in &adj_list[v] {
            if colors[u] >= 0 {
                used.insert(colors[u]);
            }
        }
        let mut c = 0i64;
        while used.contains(&c) {
            c += 1;
        }
        colors[v] = c;
    }
    colors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(a: u32, b: u32) -> CzGate {
        CzGate {
            a: AtomId(a),
            b: AtomId(b),
        }
    }

    #[test]
    fn steane_code_prep_solves_optimally() {
        // Steane [[7,1,3]] code logical-zero prep: 7 qubits, 9 CZs.
        let gates = vec![
            gate(0, 1),
            gate(0, 2),
            gate(0, 3),
            gate(1, 4),
            gate(1, 5),
            gate(2, 4),
            gate(2, 6),
            gate(3, 5),
            gate(3, 6),
        ];

        let result = schedule_exact(&gates, ExactStatePrepParams::default()).unwrap();
        assert_eq!(result.outcome, SolverOutcome::Proven);

        // The conflict graph has max degree 3, so chromatic number ≥ 3.
        // A 3-colouring exists, so the optimum is 3.
        assert_eq!(
            result.stage_count, 3,
            "Steane prep should schedule in 3 stages, got {}",
            result.stage_count
        );

        // Verify: no two gates in the same stage share an atom.
        for layer in &result.layers {
            let mut atoms = std::collections::BTreeSet::new();
            for action in &layer.actions {
                if let NeutralAtomAction::Entangle2 { atoms: [a, b], .. } = action {
                    assert!(atoms.insert(*a), "atom {:?} in two gates same stage", a);
                    assert!(atoms.insert(*b), "atom {:?} in two gates same stage", b);
                }
            }
        }
    }

    #[test]
    fn empty_gate_list() {
        let result = schedule_exact(&[], ExactStatePrepParams::default()).unwrap();
        assert_eq!(result.stage_count, 0);
        assert!(result.layers.is_empty());
    }

    #[test]
    fn single_gate_one_stage() {
        let result = schedule_exact(&[gate(0, 1)], ExactStatePrepParams::default()).unwrap();
        assert_eq!(result.stage_count, 1);
        assert_eq!(result.layers.len(), 1);
    }

    #[test]
    fn chain_gates_need_2_stages() {
        // Gates sharing atoms in a chain: (0,1), (1,2), (2,3).
        // Gates 0 and 2 share no atom → can share a stage.
        // Conflict graph is a path (bipartite) → chromatic number 2.
        let result = schedule_exact(
            &[gate(0, 1), gate(1, 2), gate(2, 3)],
            ExactStatePrepParams::default(),
        )
        .unwrap();
        assert_eq!(result.stage_count, 2);
    }
}
