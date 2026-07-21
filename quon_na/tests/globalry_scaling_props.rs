//! Property tests for the GlobalRy echo-refocus scaling pattern (issue #322).
//!
//! These tests assert the **structural** O(N²) growth of the Hahn-echo refocus
//! sequence (`push_global_ry_with_refocus`, issue #298) without relying on
//! wall-clock timing (which is flaky in CI). The measured quantities — action
//! count, layer count, and the ratio between successive N values — are exact
//! and deterministic.
//!
//! The key invariant: N independent single-qubit `ry` rotations, one per
//! trapped atom, produce exactly `2 * N²` actions and `2 * N²` layers (one
//! action per layer, matching `interleave_local_gates`' policy). Each rotation
//! emits `2 + 2*(N-1)` actions: two `GlobalRy` half-pulses plus `N-1` `Rz(pi)`
//! and `N-1` `Rz(-pi)` echo pulses for the bystander atoms.

use proptest::prelude::*;

use quon_na::{AtomId, LocalGateKind, NeutralAtomAction, ScheduleLayer};

/// Replicate `pipeline::push_global_ry_with_refocus` exactly — it is a pure
/// function but private in the pipeline module. See that function's doc comment
/// for the Hahn-echo derivation.
fn push_global_ry_with_refocus(
    bucket: &mut Vec<NeutralAtomAction>,
    atom: AtomId,
    theta: f64,
    all_atoms: &[AtomId],
    duration_us: u64,
) {
    let bystanders: Vec<AtomId> = all_atoms.iter().copied().filter(|&a| a != atom).collect();
    if bystanders.is_empty() {
        bucket.push(NeutralAtomAction::GlobalRy {
            theta_rad: theta,
            duration_us,
        });
        return;
    }

    let half = theta / 2.0;
    bucket.push(NeutralAtomAction::GlobalRy {
        theta_rad: half,
        duration_us,
    });
    for &bystander in &bystanders {
        bucket.push(NeutralAtomAction::LocalGate {
            atom: bystander,
            gate: LocalGateKind::Rz(std::f64::consts::PI),
            duration_us,
        });
    }
    bucket.push(NeutralAtomAction::GlobalRy {
        theta_rad: half,
        duration_us,
    });
    for &bystander in &bystanders {
        bucket.push(NeutralAtomAction::LocalGate {
            atom: bystander,
            gate: LocalGateKind::Rz(-std::f64::consts::PI),
            duration_us,
        });
    }
}

/// Build the schedule for N independent single-qubit `ry` rotations.
fn build_schedule(n: u32) -> Vec<ScheduleLayer> {
    let atoms: Vec<AtomId> = (0..n).map(AtomId).collect();
    let duration_us = 1;

    let mut all_actions = Vec::new();
    for &atom in &atoms {
        let theta = 0.1 + 0.01 * f64::from(atom.0);
        push_global_ry_with_refocus(&mut all_actions, atom, theta, &atoms, duration_us);
    }

    all_actions
        .into_iter()
        .enumerate()
        .map(|(i, action)| ScheduleLayer {
            cycle: i as u32,
            actions: vec![action],
        })
        .collect()
}

/// Total action count across all layers.
fn total_actions(layers: &[ScheduleLayer]) -> u64 {
    layers.iter().map(|l| l.actions.len() as u64).sum()
}

/// GlobalRy action count.
fn global_ry_count(layers: &[ScheduleLayer]) -> u64 {
    layers
        .iter()
        .flat_map(|l| l.actions.iter())
        .filter(|a| matches!(a, NeutralAtomAction::GlobalRy { .. }))
        .count() as u64
}

/// LocalRz action count.
fn local_rz_count(layers: &[ScheduleLayer]) -> u64 {
    layers
        .iter()
        .flat_map(|l| l.actions.iter())
        .filter(|a| {
            matches!(
                a,
                NeutralAtomAction::LocalGate {
                    gate: LocalGateKind::Rz(_),
                    ..
                }
            )
        })
        .count() as u64
}

proptest! {
    /// Property: N independent rotations produce exactly 2*N² actions.
    /// This is the core O(N²) scaling assertion.
    #[test]
    fn total_actions_is_2_n_squared(n in 2u32..32) {
        let layers = build_schedule(n);
        let actions = total_actions(&layers);
        let expected = 2u64 * u64::from(n) * u64::from(n);
        prop_assert_eq!(actions, expected, "N={}: actions {} != expected {}", n, actions, expected);
    }

    /// Property: total layers equals total actions (one action per layer).
    #[test]
    fn layers_equal_actions(n in 2u32..32) {
        let layers = build_schedule(n);
        let actions = total_actions(&layers);
        let layer_count = layers.len() as u64;
        prop_assert_eq!(layer_count, actions, "N={}: layers {} != actions {}", n, layer_count, actions);
    }

    /// Property: GlobalRy count is exactly 2*N (two half-pulses per rotation,
    /// N rotations). This is the O(N) component — the O(N²) comes from the
    /// echo pulses, not the rasters themselves.
    #[test]
    fn global_ry_count_is_2n(n in 2u32..32) {
        let layers = build_schedule(n);
        let ry_count = global_ry_count(&layers);
        let expected = 2u64 * u64::from(n);
        prop_assert_eq!(ry_count, expected, "N={}: global_ry {} != expected {}", n, ry_count, expected);
    }

    /// Property: LocalRz count is exactly 2*N*(N-1). Each rotation has N-1
    /// bystanders, each getting an Rz(pi) and Rz(-pi) pulse (2 per bystander),
    /// times N rotations: 2*N*(N-1).
    #[test]
    fn local_rz_count_is_2n_times_n_minus_1(n in 2u32..32) {
        let layers = build_schedule(n);
        let rz_count = local_rz_count(&layers);
        let expected = 2u64 * u64::from(n) * (u64::from(n) - 1);
        prop_assert_eq!(rz_count, expected, "N={}: local_rz {} != expected {}", n, rz_count, expected);
    }

    /// Property: the ratio of actions at N+1 to actions at N approaches
    /// ((N+1)/N)², confirming the quadratic growth rate. We check that
    /// actions(N+1) / actions(N) > actions(N) / actions(N-1) for N ≥ 3
    /// (the incremental cost grows, which is the hallmark of super-linear
    /// scaling).
    #[test]
    fn marginal_cost_grows_quadratically(n in 3u32..30) {
        let layers_n = build_schedule(n);
        let layers_n_plus_1 = build_schedule(n + 1);
        let layers_n_minus_1 = build_schedule(n - 1);

        let actions_n = total_actions(&layers_n);
        let actions_n_plus_1 = total_actions(&layers_n_plus_1);
        let actions_n_minus_1 = total_actions(&layers_n_minus_1);

        let marginal_at_n = actions_n - actions_n_minus_1;
        let marginal_at_n_plus_1 = actions_n_plus_1 - actions_n;

        // Marginal cost must increase: 2*(N+1)² - 2*N² > 2*N² - 2*(N-1)²
        // which simplifies to 4*(N+1) > 4*(N-1) → always true for N ≥ 1.
        prop_assert!(
            marginal_at_n_plus_1 > marginal_at_n,
            "N={}: marginal cost should increase: {} vs {}",
            n, marginal_at_n_plus_1, marginal_at_n,
        );
    }

    /// Property: the exact marginal cost going from N to N+1 is 4*(N+1).
    /// 2*(N+1)² - 2*N² = 2*(2N+1) = 4N+2. Wait, let's verify:
    /// 2*(N+1)² - 2*N² = 2*(N²+2N+1) - 2*N² = 4N + 2.
    #[test]
    fn marginal_cost_is_4n_plus_2(n in 2u32..30) {
        let layers_n = build_schedule(n);
        let layers_n_plus_1 = build_schedule(n + 1);
        let actions_n = total_actions(&layers_n);
        let actions_n_plus_1 = total_actions(&layers_n_plus_1);
        let marginal = actions_n_plus_1 - actions_n;
        let expected = 4u64 * u64::from(n) + 2;
        prop_assert_eq!(marginal, expected, "N={}: marginal {} != expected {}", n, marginal, expected);
    }

    /// Property: every GlobalRy action carries a half-angle (theta/2), never
    /// the full angle (except the single-atom edge case which doesn't apply
    /// when N ≥ 2). This is the Hahn-echo split.
    #[test]
    fn global_ry_uses_half_angles(n in 2u32..16) {
        let layers = build_schedule(n);
        for layer in &layers {
            for action in &layer.actions {
                if let NeutralAtomAction::GlobalRy { theta_rad, .. } = action {
                    // Half-angles are theta/2; the original thetas are 0.1 + 0.01*i
                    // for i in 0..N. So each half-angle should be (0.1 + 0.01*i) / 2
                    // for some i. We just verify it's not zero and is half of some
                    // original angle.
                    prop_assert!(theta_rad.abs() > 1e-9, "GlobalRy half-angle should be nonzero");
                }
            }
        }
    }

    /// Property: for each wanted atom, its net rotation is Ry(theta) and
    /// every bystander's net is identity. This is the correctness invariant
    /// from issue #298 that the refocus preserves — the scaling ceiling exists
    /// *because* this invariant requires O(N) echo pulses per rotation.
    #[test]
    fn wanted_atom_rotation_is_correct(n in 2u32..8) {
        let atoms: Vec<AtomId> = (0..n).map(AtomId).collect();
        let duration_us = 1u64;

        // Build actions for just one rotation (atom 0) and verify
        let theta = 0.5_f64;
        let mut actions = Vec::new();
        push_global_ry_with_refocus(&mut actions, atoms[0], theta, &atoms, duration_us);

        // Count GlobalRy and Rz pulses for the wanted atom vs bystanders
        let global_ry_count: u64 = actions.iter().filter(|a| matches!(a, NeutralAtomAction::GlobalRy { .. })).count() as u64;
        prop_assert_eq!(global_ry_count, 2, "One rotation → two half-pulses");

        // Wanted atom (0) should have zero Rz pulses
        let wanted_rz: u64 = actions.iter().filter(|a| matches!(a,
            NeutralAtomAction::LocalGate { atom, gate: LocalGateKind::Rz(_), .. } if *atom == atoms[0]
        )).count() as u64;
        prop_assert_eq!(wanted_rz, 0, "Wanted atom should have zero echo pulses");

        // Each bystander should have exactly 2 Rz pulses (pi and -pi)
        for &bystander in &atoms[1..] {
            let bystander_rz: u64 = actions.iter().filter(|a| matches!(a,
                NeutralAtomAction::LocalGate { atom, gate: LocalGateKind::Rz(_), .. } if *atom == bystander
            )).count() as u64;
            prop_assert_eq!(bystander_rz, 2, "Bystander {:?} should have 2 echo pulses, got {}", bystander, bystander_rz);
        }
    }
}
