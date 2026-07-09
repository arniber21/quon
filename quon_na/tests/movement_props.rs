//! Property tests for flat AOD movement (#106).

use std::collections::BTreeMap;

use proptest::prelude::*;
use quon_na::{
    AtomId, MovementParams, NeutralAtomAction, PlacementStrategy, Position, ResourceReport,
    SITE_PITCH_UM, SiteId, TransferDirection, TrapBinding, check_entangling_geometry,
    cubic_commutation_graph, erdos_renyi_commutation_graph, euclidean_um, place, plan_aod_movement,
    schedule_entangling_layers, schedule_from_graph, verify_aod_legality,
};

/// Pitch > min_rydberg_spacing so idle–idle R3 can pass (no spectator parking).
const ISOLATED_PITCH_UM: f64 = 20.0;

fn er_edges(n: u32, seed: u64) -> Vec<(u32, u32)> {
    let mut edges = Vec::new();
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    for a in 0..n {
        for b in (a + 1)..n {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            if (state % 100) < 35 {
                edges.push((a, b));
            }
        }
    }
    if edges.is_empty() && n >= 2 {
        edges.push((0, 1));
    }
    edges
}

fn stretch_occupied_isolated(req: &mut quon_na::GraphScheduleRequest) {
    let layout = req.layout.as_mut().expect("layout");
    let bindings: Vec<(AtomId, SiteId)> = layout
        .initial_bindings
        .iter()
        .map(|b| {
            let site = match b.trap {
                TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
            };
            (b.atom, site)
        })
        .collect();
    let n = bindings.len().max(1);
    let cols = (n as f64).sqrt().ceil() as usize;
    for (i, (_atom, site)) in bindings.iter().enumerate() {
        let row = i / cols;
        let col = i % cols;
        if let Some(s) = layout.sites.iter_mut().find(|s| s.id == *site) {
            s.position = Position {
                x_um: (col as f64) * ISOLATED_PITCH_UM,
                y_um: (row as f64) * ISOLATED_PITCH_UM,
            };
        }
    }
}

fn plan_random(seed: u64, n: u32, er: bool) -> Option<quon_na::MovementPlanResult> {
    let graph = if er {
        let edges = er_edges(n, seed);
        erdos_renyi_commutation_graph(n, &edges).ok()?
    } else {
        cubic_commutation_graph(n).ok()?
    };
    if graph.edges.is_empty() {
        return None;
    }
    let req = schedule_from_graph(graph).ok()?;
    let placed = place(req, PlacementStrategy::RowMajor).ok()?;
    let mut scheduled = schedule_entangling_layers(placed.request, 340)
        .ok()?
        .request;
    stretch_occupied_isolated(&mut scheduled);
    plan_aod_movement(scheduled, &MovementParams::generic_rna_v0()).ok()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn occupancy_and_conflicts_ok(seed in 0u64..200, n in 4u32..12) {
        let Some(result) = plan_random(seed, n, true) else { return Ok(()); };
        for layer in &result.request.layers {
            prop_assert!(layer.validate_occupancy().is_ok());
            prop_assert!(layer.validate_conflicts().is_ok());
        }
    }

    #[test]
    fn partners_and_isolation_before_entangle(seed in 0u64..200, n in 4u32..10) {
        let Some(result) = plan_random(seed, n, true) else { return Ok(()); };
        let layout = result.request.layout.as_ref().expect("layout");
        let site_pos: BTreeMap<SiteId, Position> =
            layout.sites.iter().map(|s| (s.id, s.position)).collect();
        let p = MovementParams::generic_rna_v0();

        // Replay occupancy from pre-plan bindings reconstructed via reverse moves,
        // then forward through layers — seed from final then rewind moves.
        let mut occ_atom: BTreeMap<AtomId, SiteId> = layout
            .initial_bindings
            .iter()
            .map(|b| {
                let site = match b.trap {
                    TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
                };
                (b.atom, site)
            })
            .collect();
        // Final bindings are post-plan; rewind AtomMoves to recover start, then replay.
        let mut initial = occ_atom.clone();
        for layer in result.request.layers.iter().rev() {
            for action in &layer.actions {
                if let NeutralAtomAction::Move(g) = action {
                    for m in &g.moves {
                        initial.insert(m.atom, m.from);
                    }
                }
            }
        }
        occ_atom = initial;
        let mut occ_site: BTreeMap<SiteId, AtomId> =
            occ_atom.iter().map(|(&a, &s)| (s, a)).collect();

        for layer in &result.request.layers {
            // Replay non-entangle actions, then check once with the full partner
            // set for this layer (matches planner B11 — co-scheduled pairs are partners).
            let mut layer_gates: Vec<(AtomId, AtomId)> = Vec::new();
            for action in &layer.actions {
                match action {
                    NeutralAtomAction::Transfer(t) => {
                        if t.direction == TransferDirection::SlmToAod {
                            if occ_site.get(&t.site) == Some(&t.atom) {
                                occ_site.remove(&t.site);
                            }
                        } else {
                            occ_site.insert(t.site, t.atom);
                            occ_atom.insert(t.atom, t.site);
                        }
                    }
                    NeutralAtomAction::Move(g) => {
                        for m in &g.moves {
                            occ_atom.insert(m.atom, m.to);
                        }
                    }
                    NeutralAtomAction::Entangle2 { atoms, .. } => {
                        layer_gates.push((atoms[0], atoms[1]));
                    }
                    _ => {}
                }
            }
            if !layer_gates.is_empty() {
                prop_assert!(
                    check_entangling_geometry(
                        layer.cycle,
                        &layer_gates,
                        &occ_atom,
                        &site_pos,
                        &p
                    )
                    .is_ok()
                );
                for &(a, b) in &layer_gates {
                    let sa = occ_atom[&a];
                    let sb = occ_atom[&b];
                    let d = euclidean_um(site_pos[&sa], site_pos[&sb]);
                    prop_assert!(d <= p.rydberg_range_um);
                }
            }
        }
    }

    #[test]
    fn verifier_accepts_all_groups(seed in 0u64..150, n in 4u32..10) {
        let Some(result) = plan_random(seed, n, true) else { return Ok(()); };
        let layout = result.request.layout.as_ref().unwrap();
        let mut yv: Vec<f64> = layout.sites.iter().map(|s| s.position.y_um).collect();
        let mut xv: Vec<f64> = layout.sites.iter().map(|s| s.position.x_um).collect();
        yv.sort_by(|a, b| a.total_cmp(b));
        xv.sort_by(|a, b| a.total_cmp(b));
        yv.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        xv.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let p = MovementParams::generic_rna_v0();
        for layer in &result.request.layers {
            for action in &layer.actions {
                if let NeutralAtomAction::Move(g) = action {
                    let mut specs = Vec::new();
                    for m in &g.moves {
                        let from = layout.sites.iter().find(|s| s.id == m.from).unwrap();
                        let to = layout.sites.iter().find(|s| s.id == m.to).unwrap();
                        let row = yv
                            .iter()
                            .position(|y| (*y - from.position.y_um).abs() < 1e-9)
                            .unwrap() as u32;
                        let col = xv
                            .iter()
                            .position(|x| (*x - from.position.x_um).abs() < 1e-9)
                            .unwrap() as u32;
                        specs.push(quon_na::MoveSpec {
                            atom: m.atom.0,
                            from_site: m.from.0,
                            to_site: m.to.0,
                            aod_id: 0,
                            row,
                            col,
                            from_x_um: from.position.x_um,
                            from_y_um: from.position.y_um,
                            to_x_um: to.position.x_um,
                            to_y_um: to.position.y_um,
                        });
                    }
                    prop_assert!(verify_aod_legality(layer.cycle, &specs, p.min_row_col_separation_um).is_ok());
                }
            }
        }
    }

    #[test]
    fn cost_matches_report(seed in 0u64..100, n in 4u32..10) {
        let Some(result) = plan_random(seed, n, false) else { return Ok(()); };
        let report = ResourceReport::from_layers(&result.request.layers);
        prop_assert_eq!(report.rearrangement_steps, result.rearrangement_steps);
        prop_assert_eq!(report.trap_transfers, result.trap_transfers);
        let _ = SITE_PITCH_UM;
    }

    #[test]
    fn skip_count_bounded(seed in 0u64..100, n in 4u32..10) {
        let Some(result) = plan_random(seed, n, true) else { return Ok(()); };
        let entangles = result
            .request
            .layers
            .iter()
            .flat_map(|l| l.actions.iter())
            .filter(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
            .count() as u64;
        prop_assert!(result.skipped_already_adjacent <= entangles);
    }
}
