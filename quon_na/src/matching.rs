//! In-crate min-weight bipartite matching (issue #300).
//!
//! Assigns `n` gates to `n` distinct entanglement pairs out of `m >= n`
//! candidates at minimum total travel distance — the routing-agnostic
//! [`crate::zoned`] analogue of mqt-qmap's `VertexMatchingPlacer` (Lin et al.
//! 2025). Pure Rust, no external dependency (per the #300 maintainer decision:
//! the matching lives in-crate, not in `petgraph` or a Hungarian crate).
//!
//! # Algorithm
//!
//! The O(n²·m) augmenting-path Kuhn–Munkres (Hungarian) algorithm with
//! potentials — the CP-algorithms / e-maxx formulation, restricted to the
//! rectangular `n_rows <= n_cols` case the placer needs (the schedule already
//! rejects `gates > pairs` via [`crate::zoned::ZonedScheduleError::InsufficientPairs`]
//! before matching runs). Square matrices are the `n_rows == n_cols` special
//! case. Because the outer loop runs once per *row* and the augmenting-path
//! inner scan is over *columns*, the cost is `O(n_rows² · n_cols)`, **not**
//! `O(n_cols³)`: a 21-gate / 340-pair ising_n42 layer is ~150 k operations, not
//! ~39 M, so the matching placer runs in well under a millisecond even in debug.
//!
//! # Forbidden entries
//!
//! Occupancy-illegal and repair-forbidden `(gate, pair)` edges are encoded as
//! [`FORBIDDEN_COST`]: a *finite* sentinel larger than any real travel distance
//! (µm sums are at most a few thousand on the anchored fixtures, so `1e15`
//! leaves ~12 orders of headroom). Finiteness is load-bearing — `f64::INFINITY`
//! would make the potential updates `∞ − ∞ = NaN` and corrupt the assignment.
//! Gates whose only remaining entries are forbidden are still assigned some
//! column (the algorithm is a perfect matching on rows); the placer detects
//! "no legal pair" by re-checking legality of the matched entry and deferring.

/// Finite sentinel for forbidden cost-matrix entries (occupancy-illegal or
/// repair-forbidden). See the module doc for why this is finite, not infinity.
pub(crate) const FORBIDDEN_COST: f64 = 1e15;

/// Minimum-weight assignment of `n_rows` rows to `n_rows` distinct columns out
/// of `n_cols >= n_rows`. `cost[i][j]` is the cost of row `i` → column `j`.
/// Returns `result` with `result[i]` = the column (`0..n_cols`) assigned to row
/// `i`; the `result` is an injection (distinct columns).
///
/// Every row is matched to *some* column even if all its entries are
/// [`FORBIDDEN_COST`] — callers re-check `cost[i][result[i]]` to detect an
/// effectively-unmatched (no-legal-pair) row.
///
/// O(n_rows² · n_cols). See the module doc.
pub(crate) fn min_weight_assignment(cost: &[Vec<f64>]) -> Vec<usize> {
    let n = cost.len(); // rows
    if n == 0 {
        return Vec::new();
    }
    let m = cost[0].len(); // cols, m >= n
    debug_assert!(m >= n, "min_weight_assignment requires n_cols >= n_rows");
    debug_assert!(cost.iter().all(|r| r.len() == m));

    // 1-indexed potentials: u over rows, v over columns. p[j] = row matched to
    // column j (0 = free); way[j] = predecessor column on the augmenting path.
    let mut u = vec![0.0_f64; n + 1];
    let mut v = vec![0.0_f64; m + 1];
    let mut p = vec![0_usize; m + 1];
    let mut way = vec![0_usize; m + 1];

    for i in 1..=n {
        p[0] = i; // virtual column 0 "matched" to the row being augmented
        let mut j0: usize = 0;
        let mut minv = vec![f64::INFINITY; m + 1];
        let mut used = vec![false; m + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = f64::INFINITY;
            let mut j1: usize = 0;
            for j in 1..=m {
                if !used[j] {
                    let cur = cost[i0 - 1][j - 1] - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=m {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break; // reached a free column
            }
        }
        // Trace the augmenting path back, reassigning columns.
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }

    let mut result = vec![0_usize; n];
    for j in 1..=m {
        if p[j] != 0 {
            result[p[j] - 1] = j - 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_matrix_returns_empty() {
        assert!(min_weight_assignment(&[]).is_empty());
    }

    #[test]
    fn single_row_picks_cheapest_column() {
        let cost = vec![vec![5.0, 2.0, 9.0]];
        assert_eq!(min_weight_assignment(&cost), vec![1]);
    }

    #[test]
    fn square_two_by_two_picks_global_optimum() {
        // Greedy row-by-row (row 0 → col 0, then row 1 → col 1) costs 1 + 100
        // = 101; the optimum swaps: row 0 → col 1 (2), row 1 → col 0 (2),
        // total 4. The matching must find the swap, not the myopic pick.
        let cost = vec![vec![1.0, 2.0], vec![2.0, 100.0]];
        assert_eq!(min_weight_assignment(&cost), vec![1, 0]);
    }

    #[test]
    fn rectangular_more_cols_than_rows() {
        // Two rows, three cols. Row 0's cheapest is col 0 (1), but row 1's only
        // good option is also col 0 (1) — col 1/2 are bad for row 1. Optimum:
        // row 0 → col 1 (2), row 1 → col 0 (1), total 3 < greedy's 1+50=51.
        let cost = vec![vec![1.0, 2.0, 3.0], vec![1.0, 50.0, 50.0]];
        let r = min_weight_assignment(&cost);
        assert_eq!(r.len(), 2);
        assert_ne!(r[0], r[1], "columns must be distinct");
        assert_eq!(r[1], 0, "row 1 must take col 0 (its only cheap option)");
        assert_eq!(r[0], 1, "row 0 yields col 0 to row 1, takes col 1");
    }

    #[test]
    fn assignment_is_an_injection() {
        // Random-ish 4×6 matrix: no two rows may share a column.
        let cost = vec![
            vec![10.0, 1.0, 8.0, 7.0, 6.0, 5.0],
            vec![1.0, 9.0, 2.0, 8.0, 7.0, 6.0],
            vec![8.0, 7.0, 1.0, 9.0, 3.0, 8.0],
            vec![7.0, 8.0, 9.0, 1.0, 8.0, 4.0],
        ];
        let r = min_weight_assignment(&cost);
        assert_eq!(r.len(), 4);
        let mut cols: Vec<usize> = r.clone();
        cols.sort();
        cols.dedup();
        assert_eq!(cols.len(), 4, "distinct columns: {r:?}");
    }

    #[test]
    fn forbidden_entries_are_avoided_when_legal_alternatives_exist() {
        // Row 0 cannot use col 0 (forbidden); it must take col 1. Row 1 takes
        // col 0. Both rows have a finite option, so no deferral.
        let cost = vec![vec![FORBIDDEN_COST, 2.0], vec![1.0, FORBIDDEN_COST]];
        assert_eq!(min_weight_assignment(&cost), vec![1, 0]);
    }

    #[test]
    fn all_forbidden_row_still_returns_a_column() {
        // Row 1 has no legal pair (all forbidden). It is still assigned a
        // distinct column (the caller defers it by re-checking legality); row 0
        // keeps its legal column 0. Finiteness of FORBIDDEN_COST keeps the
        // potential arithmetic NaN-free here.
        let cost = vec![
            vec![1.0, FORBIDDEN_COST],
            vec![FORBIDDEN_COST, FORBIDDEN_COST],
        ];
        let r = min_weight_assignment(&cost);
        assert_eq!(r.len(), 2);
        assert_ne!(r[0], r[1]);
        assert_eq!(r[0], 0, "row 0 keeps its only legal column");
    }

    #[test]
    fn optimum_matches_brute_force_on_small_random_matrix() {
        // Cross-check against exhaustive enumeration on a 4×4 matrix.
        let cost = vec![
            vec![9.0, 2.0, 7.0, 8.0],
            vec![6.0, 4.0, 3.0, 5.0],
            vec![5.0, 8.0, 1.0, 7.0],
            vec![3.0, 9.0, 6.0, 2.0],
        ];
        let r = min_weight_assignment(&cost);
        let got: f64 = (0..4).map(|i| cost[i][r[i]]).sum();
        // Brute-force min over all permutations of 4 columns.
        let cols = [0, 1, 2, 3];
        let mut best = f64::INFINITY;
        permute(&cols, &mut best, &cost);
        assert!(
            (got - best).abs() < 1e-9,
            "matching cost {got} != brute-force optimum {best} (assignment {r:?})"
        );
    }

    /// Exhaustive min-cost over all column permutations (n! only feasible for n<=4).
    fn permute(cols: &[usize; 4], best: &mut f64, cost: &[Vec<f64>]) {
        let mut a = *cols;
        permute_rec(&mut a, 0, best, cost);
    }

    fn permute_rec(a: &mut [usize; 4], k: usize, best: &mut f64, cost: &[Vec<f64>]) {
        if k == 4 {
            let total: f64 = (0..4).map(|i| cost[i][a[i]]).sum();
            if total < *best {
                *best = total;
            }
            return;
        }
        for i in k..4 {
            a.swap(k, i);
            permute_rec(a, k + 1, best, cost);
            a.swap(k, i);
        }
    }
}
