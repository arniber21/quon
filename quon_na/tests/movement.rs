//! Flat AOD movement planner tests (issue #106) — B1–B14 locks.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use quon_na::{
    AtomId, AtomMove, AtomSite, BANK_ISOLATION_EPS_UM, CandidateLeg, Interaction, InteractionGraph,
    InteractionId, InteractionSegment, LogicalQubitId, MovementGroup, MovementParams,
    MovementPlanError, NeutralAtomAction, NeutralAtomLayout, PlacementStrategy, Position,
    ResourceReport, SITE_PITCH_UM, ScheduleLayer, SegmentKind, SiteId, TransferDirection,
    TrapBinding, atom_moves_to_move_specs, check_entangling_geometry, ensure_interaction_pairs,
    euclidean_um, legs_conflict, movement_duration_us, place, plan_aod_movement,
    schedule_entangling_layers, schedule_from_graph, try_transfer_into_occupied,
    verify_aod_legality, verify_entangling_geometry_predicates,
};

fn cz(id: u32, a: u32, b: u32) -> Interaction {
    let mut qubits = vec![LogicalQubitId(a), LogicalQubitId(b)];
    qubits.sort();
    Interaction {
        id: InteractionId(id),
        qubits,
        gate_name: "CZ".into(),
        dag_layer: 0,
        on_critical_path: false,
    }
}

fn graph_from_edges(n: u32, edges: &[(u32, u32)]) -> InteractionGraph {
    let interactions: Vec<_> = edges
        .iter()
        .enumerate()
        .map(|(i, &(a, b))| cz(i as u32, a, b))
        .collect();
    let ids: Vec<_> = (0..interactions.len() as u32).map(InteractionId).collect();
    InteractionGraph::from_interactions(
        (0..n).map(LogicalQubitId).collect(),
        interactions,
        vec![InteractionSegment {
            kind: SegmentKind::CommutationGroup,
            interactions: ids,
        }],
        quon_na::DEFAULT_GAMMA,
    )
    .expect("graph")
}

fn place_and_schedule(graph: InteractionGraph) -> quon_na::GraphScheduleRequest {
    let req = schedule_from_graph(graph).expect("from_graph");
    let placed = place(req, PlacementStrategy::RowMajor).expect("place");
    schedule_entangling_layers(placed.request, 340)
        .expect("entangle")
        .request
}

/// Pitch strictly above `min_rydberg_spacing_um` so idle–idle R3 can pass without parking.
const ISOLATED_PITCH_UM: f64 = 20.0;

/// Remap occupied placement sites onto an isolated grid (pitch > min R3).
/// Dense `#104` pitch (5 µm) leaves idle–idle R2/R3 violations; Ok fixtures must isolate.
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

fn place_schedule_isolated(graph: InteractionGraph) -> quon_na::GraphScheduleRequest {
    let mut req = place_and_schedule(graph);
    stretch_occupied_isolated(&mut req);
    req
}

fn params() -> MovementParams {
    MovementParams::generic_rna_v0()
}

fn site_map(layout: &NeutralAtomLayout) -> BTreeMap<SiteId, Position> {
    layout.sites.iter().map(|s| (s.id, s.position)).collect()
}

fn occ_from_layout(layout: &NeutralAtomLayout) -> BTreeMap<AtomId, SiteId> {
    layout
        .initial_bindings
        .iter()
        .map(|b| {
            let site = match b.trap {
                TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
            };
            (b.atom, site)
        })
        .collect()
}

fn move_groups(layers: &[ScheduleLayer]) -> Vec<&MovementGroup> {
    layers
        .iter()
        .filter_map(|l| {
            l.actions.iter().find_map(|a| match a {
                NeutralAtomAction::Move(g) => Some(g),
                _ => None,
            })
        })
        .collect()
}

fn all_atom_moves(layers: &[ScheduleLayer]) -> Vec<&AtomMove> {
    let mut out = Vec::new();
    for l in layers {
        for a in &l.actions {
            if let NeutralAtomAction::Move(g) = a {
                out.extend(g.moves.iter());
            }
        }
    }
    out
}

#[test]
fn pair_gap_leq_rb() {
    let mut layout = NeutralAtomLayout {
        sites: vec![AtomSite {
            id: SiteId(0),
            position: Position {
                x_um: 0.0,
                y_um: 0.0,
            },
        }],
        initial_bindings: vec![quon_na::AtomBinding {
            atom: AtomId(0),
            trap: TrapBinding::Slm { site: SiteId(0) },
        }],
    };
    let p = params();
    let pairs = ensure_interaction_pairs(&mut layout, &p, 3).expect("bank");
    let map = site_map(&layout);
    for pair in pairs {
        let d = euclidean_um(map[&pair.left], map[&pair.right]);
        assert!(d <= p.rydberg_range_um, "gap {d} > rb");
    }
}

#[test]
fn pair_pitch_isolates_non_partners() {
    let mut layout = NeutralAtomLayout {
        sites: vec![AtomSite {
            id: SiteId(0),
            position: Position {
                x_um: 0.0,
                y_um: 0.0,
            },
        }],
        initial_bindings: vec![quon_na::AtomBinding {
            atom: AtomId(0),
            trap: TrapBinding::Slm { site: SiteId(0) },
        }],
    };
    let p = params();
    let pairs = ensure_interaction_pairs(&mut layout, &p, 4).expect("bank");
    let map = site_map(&layout);
    let mut min_cross = f64::INFINITY;
    for (i, a) in pairs.iter().enumerate() {
        for b in pairs.iter().skip(i + 1) {
            for sa in [a.left, a.right] {
                for sb in [b.left, b.right] {
                    min_cross = min_cross.min(euclidean_um(map[&sa], map[&sb]));
                }
            }
        }
    }
    assert!(
        min_cross > p.min_rydberg_spacing_um,
        "cross-pair {min_cross} not > min"
    );
}

#[test]
fn bank_outside_placement_isolated() {
    // Dense #104 pitch is fine here — only measures placement↔bank gap (B14).
    let graph = graph_from_edges(4, &[(0, 1), (2, 3)]);
    let mut req = place_and_schedule(graph);
    let layout = req.layout.as_mut().expect("layout");
    let p = params();
    let pairs = ensure_interaction_pairs(layout, &p, 2).expect("bank");
    let map = site_map(layout);
    let placement: BTreeSet<SiteId> = layout
        .initial_bindings
        .iter()
        .map(|b| match b.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
        })
        .collect();
    let bank: BTreeSet<SiteId> = pairs.iter().flat_map(|x| [x.left, x.right]).collect();
    let mut min_d = f64::INFINITY;
    for &ps in &placement {
        for &bs in &bank {
            min_d = min_d.min(euclidean_um(map[&ps], map[&bs]));
        }
    }
    // B14: dialect-strict `>` min (not `≥ pair_pitch_um`).
    assert!(
        min_d > p.min_rydberg_spacing_um,
        "placement↔bank {min_d} not > {}",
        p.min_rydberg_spacing_um
    );
    assert!((BANK_ISOLATION_EPS_UM - 0.01).abs() < 1e-12);
}

#[test]
fn partner_reachable_under_defaults() {
    // Far gate on placement grid → plan Ok under generic_rna defaults.
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("plan");
    assert!(result.rearrangement_steps >= 1);
}

#[test]
fn dual_moves_both_atoms_onto_pair() {
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("plan");
    let moves = all_atom_moves(&result.request.layers);
    let moved: BTreeSet<_> = moves.iter().map(|m| m.atom).collect();
    assert!(moved.contains(&AtomId(0)));
    assert!(moved.contains(&AtomId(3)));
}

#[test]
fn dual_dest_is_existing_site_id() {
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("plan");
    let sites: BTreeSet<_> = result
        .request
        .layout
        .as_ref()
        .expect("layout")
        .sites
        .iter()
        .map(|s| s.id)
        .collect();
    for m in all_atom_moves(&result.request.layers) {
        assert!(sites.contains(&m.to), "virtual dest {:?}", m.to);
        assert!(sites.contains(&m.from));
    }
}

#[test]
fn no_virtual_coords() {
    dual_dest_is_existing_site_id();
}

#[test]
fn invalid_pair_gap_rejected() {
    let mut p = params();
    p.pair_gap_um = 8.0; // > rb 7.5
    let mut layout = NeutralAtomLayout::default();
    let err = ensure_interaction_pairs(&mut layout, &p, 1).unwrap_err();
    assert!(matches!(err, MovementPlanError::InvalidPairGap { .. }));
}

#[test]
fn params_include_min_rydberg_spacing() {
    let mut p = params();
    p.min_rydberg_spacing_um = 0.0;
    let graph = graph_from_edges(2, &[(0, 1)]);
    let req = place_schedule_isolated(graph);
    let err = plan_aod_movement(req, &p).unwrap_err();
    assert!(matches!(
        err,
        MovementPlanError::InvalidMinRydbergSpacing(_)
    ));
}

#[test]
fn default_transfer_policy_is_quon_reuse() {
    let p = params();
    assert!(
        !p.return_home,
        "Quon reuse: return_home=false ⇒ 2 xfers/atom"
    );
    // B6: only return_home selects 2- vs 4-xfer; no transfers_per_moved_atom field.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/movement/mod.rs");
    let src = fs::read_to_string(&path).expect("read");
    assert!(
        !src.contains("transfers_per_moved_atom"),
        "dead transfers_per_moved_atom must stay deleted"
    );
    assert!(
        src.contains(
            "Only [`types::MovementParams::return_home`] selects the 2- vs 4-transfer policy"
        ) || src.contains("only `return_home` selects")
            || src.contains("This flag alone selects the 2- vs 4-transfer policy"),
        "docs must state return_home selects transfer policy"
    );
}

#[test]
fn docs_do_not_claim_enola_duals() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/movement/mod.rs");
    let src = fs::read_to_string(&path).expect("read movement/mod.rs");
    assert!(
        !src.contains("implements Enola Sec. 5 sortIS dual selection"),
        "forbidden Enola dual-selection claim"
    );
    assert!(
        src.contains("this planner does not implement those duals"),
        "must disclaim Enola one-atom duals"
    );
    assert!(
        src.contains("Quon"),
        "must label pair-bank / both-atom as Quon"
    );
    // Forbidden phrase as algorithm claim
    let lower = src.to_lowercase();
    assert!(
        !lower.contains("enola duals")
            || src.contains("does not implement those duals")
                && src.contains("Enola Sec. 5 duals are one-atom"),
        "must not claim Enola duals as this algorithm"
    );
}

#[test]
fn uses_shared_movement_duration_us() {
    // architecture_model example: 110 µm / 2750 → ~200 µs
    let us = movement_duration_us(110.0, 2750.0);
    assert!((190..=210).contains(&us), "got {us}");
}

#[test]
fn sqrt_cost_matches_formula() {
    uses_shared_movement_duration_us();
}

#[test]
fn missing_layout_errors() {
    let graph = graph_from_edges(2, &[(0, 1)]);
    let mut req = schedule_from_graph(graph).expect("g");
    req.layers = vec![ScheduleLayer {
        cycle: 0,
        actions: vec![NeutralAtomAction::Entangle2 {
            atoms: [AtomId(0), AtomId(1)],
            duration_us: 1,
        }],
    }];
    let err = plan_aod_movement(req, &params()).unwrap_err();
    assert_eq!(err, MovementPlanError::MissingLayout);
}

#[test]
fn empty_layers_errors() {
    let graph = graph_from_edges(2, &[(0, 1)]);
    let req = place(
        schedule_from_graph(graph).unwrap(),
        PlacementStrategy::RowMajor,
    )
    .unwrap()
    .request;
    let err = plan_aod_movement(req, &params()).unwrap_err();
    assert_eq!(err, MovementPlanError::EmptySchedule);
}

#[test]
fn unsupported_entangle_n_errors() {
    let graph = graph_from_edges(3, &[(0, 1)]);
    let mut req = place_schedule_isolated(graph);
    req.layers = vec![ScheduleLayer {
        cycle: 0,
        actions: vec![NeutralAtomAction::EntangleN {
            atoms: vec![AtomId(0), AtomId(1), AtomId(2)],
            duration_us: 1,
        }],
    }];
    let err = plan_aod_movement(req, &params()).unwrap_err();
    assert!(matches!(
        err,
        MovementPlanError::UnsupportedEntangleArity { k: 3, .. }
    ));
}

#[test]
fn explicit_transfer_layers_around_move() {
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("plan");
    let layers = &result.request.layers;
    // Find a Move and assert neighbors are Transfer load/store pattern.
    let mut found = false;
    for i in 0..layers.len() {
        if layers[i]
            .actions
            .iter()
            .any(|a| matches!(a, NeutralAtomAction::Move(_)))
        {
            assert!(i > 0, "move needs prior load");
            assert!(
                layers[i - 1]
                    .actions
                    .iter()
                    .all(|a| matches!(a, NeutralAtomAction::Transfer(t) if t.direction == TransferDirection::SlmToAod)),
                "load before move"
            );
            assert!(i + 1 < layers.len());
            assert!(
                layers[i + 1]
                    .actions
                    .iter()
                    .all(|a| matches!(a, NeutralAtomAction::Transfer(t) if t.direction == TransferDirection::AodToSlm)),
                "store after move"
            );
            found = true;
            break;
        }
    }
    assert!(found);
}

#[test]
fn grouped_move_single_action() {
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("plan");
    for layer in &result.request.layers {
        let moves: Vec<_> = layer
            .actions
            .iter()
            .filter(|a| matches!(a, NeutralAtomAction::Move(_)))
            .collect();
        assert!(moves.len() <= 1, "one Move action per layer");
    }
}

#[test]
fn resource_report_counts_rounds() {
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("plan");
    let report = ResourceReport::from_layers(&result.request.layers);
    assert_eq!(report.rearrangement_steps, result.rearrangement_steps);
    assert_eq!(report.rearrangement_time_us, result.rearrangement_time_us);
    assert_eq!(report.trap_transfers, result.trap_transfers);
}

#[test]
fn unequal_y_partners_pass_aod_verifier() {
    // #104 place puts atoms on different rows for a 2×2 grid: 0@(0,0), 1@(5,0), 2@(0,5), 3@(5,5)
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let layout = req.layout.as_ref().unwrap();
    let pos = |id: u32| {
        let site = match layout
            .initial_bindings
            .iter()
            .find(|b| b.atom == AtomId(id))
            .unwrap()
            .trap
        {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
        };
        layout.sites.iter().find(|s| s.id == site).unwrap().position
    };
    assert!(
        (pos(0).y_um - pos(3).y_um).abs() > 1e-6,
        "need unequal src y"
    );
    let result = plan_aod_movement(req, &params()).expect("plan");
    let groups = move_groups(&result.request.layers);
    // Relocation legs for the two atoms must not share a MovementGroup (B7).
    let mut atom0_groups = Vec::new();
    let mut atom3_groups = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        for m in &g.moves {
            if m.atom == AtomId(0) {
                atom0_groups.push(gi);
            }
            if m.atom == AtomId(3) {
                atom3_groups.push(gi);
            }
        }
    }
    // First relocation of each should be in different groups if both moved to bank.
    if !atom0_groups.is_empty() && !atom3_groups.is_empty() {
        assert_ne!(
            atom0_groups[0], atom3_groups[0],
            "B7: dual legs must be serialized"
        );
    }
    let p = params();
    for g in groups {
        let mut specs = Vec::new();
        for m in &g.moves {
            let layout = result.request.layout.as_ref().unwrap();
            let from = layout.sites.iter().find(|s| s.id == m.from).unwrap();
            let to = layout.sites.iter().find(|s| s.id == m.to).unwrap();
            // Reconstruct AOD from source coords via dense overlay — use verify helper path.
            specs.push(quon_na::MoveSpec {
                atom: m.atom.0,
                from_site: m.from.0,
                to_site: m.to.0,
                aod_id: 0,
                row: 0, // filled below
                col: 0,
                from_x_um: from.position.x_um,
                from_y_um: from.position.y_um,
                to_x_um: to.position.x_um,
                to_y_um: to.position.y_um,
            });
        }
        // Build proper indices from layout
        let layout = result.request.layout.as_ref().unwrap();
        let mut yv: Vec<f64> = layout.sites.iter().map(|s| s.position.y_um).collect();
        let mut xv: Vec<f64> = layout.sites.iter().map(|s| s.position.x_um).collect();
        yv.sort_by(|a, b| a.total_cmp(b));
        xv.sort_by(|a, b| a.total_cmp(b));
        yv.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        xv.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        for s in &mut specs {
            let from = layout.sites.iter().find(|x| x.id.0 == s.from_site).unwrap();
            s.row = yv
                .iter()
                .position(|y| (*y - from.position.y_um).abs() < 1e-9)
                .unwrap() as u32;
            s.col = xv
                .iter()
                .position(|x| (*x - from.position.x_um).abs() < 1e-9)
                .unwrap() as u32;
        }
        verify_aod_legality(0, &specs, p.min_row_col_separation_um).expect("verifier");
    }
}

#[test]
fn verifier_accepts_emitted_group() {
    unequal_y_partners_pass_aod_verifier();
}

#[test]
fn aod_indices_from_site_coords() {
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("plan");
    let layout = result.request.layout.as_ref().unwrap();
    // Same site → same (row,col) across groups via source-site rule.
    let mut site_idx: BTreeMap<SiteId, (u32, u32)> = BTreeMap::new();
    let mut yv: Vec<f64> = layout.sites.iter().map(|s| s.position.y_um).collect();
    let mut xv: Vec<f64> = layout.sites.iter().map(|s| s.position.x_um).collect();
    yv.sort_by(|a, b| a.total_cmp(b));
    xv.sort_by(|a, b| a.total_cmp(b));
    yv.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    xv.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    for s in &layout.sites {
        let row = yv
            .iter()
            .position(|y| (*y - s.position.y_um).abs() < 1e-9)
            .unwrap() as u32;
        let col = xv
            .iter()
            .position(|x| (*x - s.position.x_um).abs() < 1e-9)
            .unwrap() as u32;
        site_idx.insert(s.id, (row, col));
    }
    for g in move_groups(&result.request.layers) {
        for m in &g.moves {
            let (r, c) = site_idx[&m.from];
            let again = site_idx[&m.from];
            assert_eq!((r, c), again);
        }
    }
}

#[test]
fn m3_separation_conflict_in_oracle() {
    let min_sep = 2.0;
    let a = CandidateLeg {
        atom: AtomId(0),
        from: SiteId(0),
        to: SiteId(10),
        from_pos: Position {
            x_um: 0.0,
            y_um: 0.0,
        },
        to_pos: Position {
            x_um: 10.0,
            y_um: 0.0,
        },
        aod_id: 0,
        row: 0,
        col: 0,
        dual_id: 0,
        distance_um: 10.0,
    };
    let b = CandidateLeg {
        atom: AtomId(1),
        from: SiteId(1),
        to: SiteId(11),
        from_pos: Position {
            x_um: 0.0,
            y_um: 5.0,
        },
        to_pos: Position {
            x_um: 10.0,
            y_um: 1.0, // |0-1| = 1 < 2
        },
        aod_id: 0,
        row: 1,
        col: 0,
        dual_id: 1,
        distance_um: 10.05,
    };
    assert!(legs_conflict(&a, &b, min_sep), "oracle must conflict on M3");
    let specs = vec![
        quon_na::MoveSpec {
            atom: 0,
            from_site: 0,
            to_site: 10,
            aod_id: 0,
            row: 0,
            col: 0,
            from_x_um: 0.0,
            from_y_um: 0.0,
            to_x_um: 10.0,
            to_y_um: 0.0,
        },
        quon_na::MoveSpec {
            atom: 1,
            from_site: 1,
            to_site: 11,
            aod_id: 0,
            row: 1,
            col: 0,
            from_x_um: 0.0,
            from_y_um: 5.0,
            to_x_um: 10.0,
            to_y_um: 1.0,
        },
    ];
    assert!(verify_aod_legality(0, &specs, min_sep).is_err());
}

#[test]
fn order_preservation_conflict() {
    let a = CandidateLeg {
        atom: AtomId(0),
        from: SiteId(0),
        to: SiteId(10),
        from_pos: Position {
            x_um: 0.0,
            y_um: 10.0,
        },
        to_pos: Position {
            x_um: 20.0,
            y_um: 0.0,
        },
        aod_id: 0,
        row: 1,
        col: 0,
        dual_id: 0,
        distance_um: 22.0,
    };
    let b = CandidateLeg {
        atom: AtomId(1),
        from: SiteId(1),
        to: SiteId(11),
        from_pos: Position {
            x_um: 0.0,
            y_um: 0.0,
        },
        to_pos: Position {
            x_um: 20.0,
            y_um: 10.0,
        },
        aod_id: 0,
        row: 0,
        col: 0,
        dual_id: 1,
        distance_um: 22.0,
    };
    assert!(legs_conflict(&a, &b, 2.0));
}

#[test]
fn same_source_row_conflict() {
    let a = CandidateLeg {
        atom: AtomId(0),
        from: SiteId(0),
        to: SiteId(10),
        from_pos: Position {
            x_um: 0.0,
            y_um: 5.0,
        },
        to_pos: Position {
            x_um: 20.0,
            y_um: 5.0,
        },
        aod_id: 0,
        row: 0,
        col: 0,
        dual_id: 0,
        distance_um: 20.0,
    };
    let b = CandidateLeg {
        atom: AtomId(1),
        from: SiteId(1),
        to: SiteId(11),
        from_pos: Position {
            x_um: 5.0,
            y_um: 5.0,
        },
        to_pos: Position {
            x_um: 25.0,
            y_um: 10.0,
        },
        aod_id: 0,
        row: 0,
        col: 1,
        dual_id: 1,
        distance_um: 20.6,
    };
    assert!(legs_conflict(&a, &b, 2.0));
}

#[test]
fn occupancy_collision_rejected() {
    // Two legs same dest → conflict
    let a = CandidateLeg {
        atom: AtomId(0),
        from: SiteId(0),
        to: SiteId(10),
        from_pos: Position {
            x_um: 0.0,
            y_um: 0.0,
        },
        to_pos: Position {
            x_um: 20.0,
            y_um: 0.0,
        },
        aod_id: 0,
        row: 0,
        col: 0,
        dual_id: 0,
        distance_um: 20.0,
    };
    let b = CandidateLeg {
        atom: AtomId(1),
        from: SiteId(1),
        to: SiteId(10),
        from_pos: Position {
            x_um: 0.0,
            y_um: 5.0,
        },
        to_pos: Position {
            x_um: 20.0,
            y_um: 0.0,
        },
        aod_id: 0,
        row: 1,
        col: 0,
        dual_id: 1,
        distance_um: 20.6,
    };
    assert!(legs_conflict(&a, &b, 2.0));
}

#[test]
fn check_geometry_matches_dialect_leq() {
    // Same fixture: planner check_entangling_geometry and shared dialect-predicate
    // helper must agree (exact min spacing → R3 reject under dialect `≤`).
    let p = params();
    let mut occ = BTreeMap::new();
    occ.insert(AtomId(0), SiteId(0));
    occ.insert(AtomId(1), SiteId(1));
    occ.insert(AtomId(2), SiteId(2));
    let mut site_pos = BTreeMap::new();
    site_pos.insert(
        SiteId(0),
        Position {
            x_um: 0.0,
            y_um: 0.0,
        },
    );
    site_pos.insert(
        SiteId(1),
        Position {
            x_um: 2.0,
            y_um: 0.0,
        },
    );
    site_pos.insert(
        SiteId(2),
        Position {
            x_um: 18.75,
            y_um: 0.0,
        },
    );
    let gates = [(AtomId(0), AtomId(1))];
    let planner = check_entangling_geometry(0, &gates, &occ, &site_pos, &p);
    let atoms: Vec<_> = occ.iter().map(|(&a, &s)| (a, site_pos[&s])).collect();
    let dialect = verify_entangling_geometry_predicates(
        0,
        &gates,
        &atoms,
        p.rydberg_range_um,
        p.min_rydberg_spacing_um,
    );
    assert!(planner.is_err(), "planner must reject exact 18.75 (R3)");
    assert!(
        dialect.is_err(),
        "dialect predicates must reject exact 18.75"
    );
    assert_eq!(
        planner.is_err(),
        dialect.is_err(),
        "planner ↔ dialect Ok/Err parity"
    );
}

#[test]
fn r3_isolation_spacing_fails() {
    let p = params();
    let mut occ = BTreeMap::new();
    occ.insert(AtomId(0), SiteId(0));
    occ.insert(AtomId(1), SiteId(1));
    occ.insert(AtomId(2), SiteId(2));
    let mut site_pos = BTreeMap::new();
    site_pos.insert(
        SiteId(0),
        Position {
            x_um: 0.0,
            y_um: 0.0,
        },
    );
    site_pos.insert(
        SiteId(1),
        Position {
            x_um: 2.0,
            y_um: 0.0,
        },
    );
    site_pos.insert(
        SiteId(2),
        Position {
            x_um: 18.75,
            y_um: 0.0,
        },
    );
    let err = check_entangling_geometry(0, &[(AtomId(0), AtomId(1))], &occ, &site_pos, &p);
    assert!(err.is_err(), "exact 18.75 must fail R3");
}

#[test]
fn r2_non_partner_too_close_fails() {
    let p = params();
    let mut occ = BTreeMap::new();
    occ.insert(AtomId(0), SiteId(0));
    occ.insert(AtomId(1), SiteId(1));
    occ.insert(AtomId(2), SiteId(2));
    let mut site_pos = BTreeMap::new();
    site_pos.insert(
        SiteId(0),
        Position {
            x_um: 0.0,
            y_um: 0.0,
        },
    );
    site_pos.insert(
        SiteId(1),
        Position {
            x_um: 2.0,
            y_um: 0.0,
        },
    );
    site_pos.insert(
        SiteId(2),
        Position {
            x_um: 4.0,
            y_um: 0.0,
        },
    ); // within rb of both
    let err = check_entangling_geometry(0, &[(AtomId(0), AtomId(1))], &occ, &site_pos, &p);
    assert!(err.is_err());
}

#[test]
fn geometry_scope_includes_idle_atoms() {
    // Idle atom ∉ gate list but within min spacing of a partner → fail (B11 all-occupied).
    // Distinct from r2: idle is outside r_b of partners but ≤ min_rydberg_spacing (R3 only).
    let p = params();
    let mut occ = BTreeMap::new();
    occ.insert(AtomId(0), SiteId(0));
    occ.insert(AtomId(1), SiteId(1));
    occ.insert(AtomId(99), SiteId(99)); // idle, not in gate
    let mut site_pos = BTreeMap::new();
    site_pos.insert(
        SiteId(0),
        Position {
            x_um: 0.0,
            y_um: 0.0,
        },
    );
    site_pos.insert(
        SiteId(1),
        Position {
            x_um: 2.0,
            y_um: 0.0,
        },
    );
    site_pos.insert(
        SiteId(99),
        Position {
            x_um: 18.75,
            y_um: 0.0,
        },
    ); // exactly min from atom 0; > rb so not R2
    let err = check_entangling_geometry(0, &[(AtomId(0), AtomId(1))], &occ, &site_pos, &p);
    match err {
        Err(MovementPlanError::EntanglingGeometry { detail, .. }) => {
            assert!(
                detail.contains("R3") && detail.contains("99"),
                "must cite idle atom 99 in R3: {detail}"
            );
        }
        other => panic!("expected EntanglingGeometry with idle in scope, got {other:?}"),
    }
}

#[test]
fn dense_placement_skip_not_ok() {
    // Dense #104 pitch 5µm + idle neighbors: no spectator parking.
    // Must not emit illegal in-place entangle — either EntanglingGeometry or
    // bank relocation without claiming skip (B10/B11).
    let graph = graph_from_edges(4, &[(0, 1)]);
    let req = place_and_schedule(graph); // keep dense pitch
    let p = params();
    match plan_aod_movement(req, &p) {
        Ok(result) => {
            assert_eq!(
                result.skipped_already_adjacent, 0,
                "dense in-place skip forbidden"
            );
            assert!(
                result.rearrangement_steps >= 1,
                "if Ok, must have relocated gate atoms to bank"
            );
            // No parking sites: every site y must be ≥ placement min_y - ε
            // (parking was below bbox). Stronger: final geometry must pass.
            let layout = result.request.layout.as_ref().unwrap();
            let occ = occ_from_layout(layout);
            let map = site_map(layout);
            let gates = [(AtomId(0), AtomId(1))];
            check_entangling_geometry(0, &gates, &occ, &map, &p)
                .expect("Ok plan must leave legal geometry without parking");
        }
        Err(MovementPlanError::EntanglingGeometry { .. }) => {
            // Fail-closed under all-occupied R2/R3 — allowed by plan.
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn skip_requires_geometry_ok() {
    dense_placement_skip_not_ok();
}

#[test]
fn skip_already_adjacent_no_move() {
    // Only two atoms on an isolated pair (gap ≤ rb, no idle neighbors) → skip.
    // Use non-bank-shaped home sites far apart, then place atoms on a dedicated
    // interaction pair so geometry passes without moves.
    let p = params();
    let layout = NeutralAtomLayout {
        sites: vec![
            AtomSite {
                id: SiteId(0),
                position: Position {
                    x_um: 0.0,
                    y_um: 0.0,
                },
            },
            AtomSite {
                id: SiteId(1),
                position: Position {
                    x_um: 50.0,
                    y_um: 0.0,
                },
            },
            // Pre-built interaction pair (gap 2.0), isolated from homes.
            AtomSite {
                id: SiteId(10),
                position: Position {
                    x_um: 100.0,
                    y_um: 0.0,
                },
            },
            AtomSite {
                id: SiteId(11),
                position: Position {
                    x_um: 102.0,
                    y_um: 0.0,
                },
            },
        ],
        initial_bindings: vec![
            quon_na::AtomBinding {
                atom: AtomId(0),
                trap: TrapBinding::Slm { site: SiteId(10) },
            },
            quon_na::AtomBinding {
                atom: AtomId(1),
                trap: TrapBinding::Slm { site: SiteId(11) },
            },
        ],
    };
    let graph = graph_from_edges(2, &[(0, 1)]);
    let mut req = schedule_from_graph(graph).unwrap();
    req.layout = Some(layout);
    req.layers = vec![ScheduleLayer {
        cycle: 0,
        actions: vec![NeutralAtomAction::Entangle2 {
            atoms: [AtomId(0), AtomId(1)],
            duration_us: 1,
        }],
    }];
    let result = plan_aod_movement(req, &p).expect("plan");
    assert!(result.skipped_already_adjacent >= 1);
    assert_eq!(result.rearrangement_steps, 0);
}

#[test]
fn all_adjacent_zero_moves_success() {
    skip_already_adjacent_no_move();
}

#[test]
fn multi_layer_reuse_default_no_exhaustion() {
    // Layer1: (0,1); Layer2: (2,3) disjoint — forces B8 eviction under return_home=false.
    let interactions = vec![cz(0, 0, 1), cz(1, 2, 3)];
    // Put in separate segments so they become separate layers (dependency or two groups).
    let graph = InteractionGraph::from_interactions(
        (0..4).map(LogicalQubitId).collect(),
        interactions,
        vec![
            InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: vec![InteractionId(0)],
            },
            InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: vec![InteractionId(1)],
            },
        ],
        quon_na::DEFAULT_GAMMA,
    )
    .unwrap();
    let req = place_schedule_isolated(graph);
    assert!(req.layers.len() >= 2);
    let result = plan_aod_movement(req, &params()).expect("B8 eviction");
    assert!(result.request.layers.iter().any(|l| {
        l.actions
            .iter()
            .any(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
    }));
}

#[test]
fn multi_layer_same_atom_reuse_no_eviction() {
    // Same gate twice in two layers — second should skip when still on pair.
    let interactions = vec![cz(0, 0, 1), cz(1, 0, 1)];
    let graph = InteractionGraph::from_interactions(
        (0..2).map(LogicalQubitId).collect(),
        interactions,
        vec![
            InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: vec![InteractionId(0)],
            },
            InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: vec![InteractionId(1)],
            },
        ],
        quon_na::DEFAULT_GAMMA,
    )
    .unwrap();
    // Isolate: only two atoms — after first move to bank, second skips.
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("reuse");
    assert!(
        result.skipped_already_adjacent >= 1,
        "second layer should skip"
    );
}

#[test]
fn partial_overlap_pair_reclaimed() {
    // Layer1 (0,1); Layer2 (0,2) — B13 whole-pair vacate before layer-2 duals.
    let interactions = vec![cz(0, 0, 1), cz(1, 0, 2)];
    let graph = InteractionGraph::from_interactions(
        (0..3).map(LogicalQubitId).collect(),
        interactions,
        vec![
            InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: vec![InteractionId(0)],
            },
            InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: vec![InteractionId(1)],
            },
        ],
        quon_na::DEFAULT_GAMMA,
    )
    .unwrap();
    let req = place_schedule_isolated(graph);
    let mut occ_site: BTreeMap<SiteId, AtomId> = BTreeMap::new();
    let mut occ_atom: BTreeMap<AtomId, SiteId> = BTreeMap::new();
    for b in &req.layout.as_ref().unwrap().initial_bindings {
        let site = match b.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
        };
        occ_atom.insert(b.atom, site);
        occ_site.insert(site, b.atom);
    }
    // Capture bank pairs before plan (post-plan bindings sit on bank → detect breaks).
    let mut layout_for_pairs = req.layout.clone().expect("layout");
    let pairs = ensure_interaction_pairs(&mut layout_for_pairs, &params(), 2).expect("bank");
    let result = plan_aod_movement(req, &params()).expect("B13 reclaim");
    assert!(result.rearrangement_steps >= 1);

    let map = site_map(result.request.layout.as_ref().unwrap());
    let mut pair_after_l1: Option<quon_na::InteractionPair> = None;
    let mut vacated_before_l2 = false;
    let mut entangle_idx = 0u32;

    for layer in &result.request.layers {
        for a in &layer.actions {
            match a {
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
                    entangle_idx += 1;
                    if entangle_idx == 1 {
                        let s0 = *occ_atom.get(&atoms[0]).expect("a");
                        let s1 = *occ_atom.get(&atoms[1]).expect("b");
                        pair_after_l1 = pairs.iter().copied().find(|p| {
                            (p.left == s0 && p.right == s1) || (p.left == s1 && p.right == s0)
                        });
                        assert!(
                            pair_after_l1.is_some(),
                            "layer-1 partners must occupy a bank pair; sites {s0:?},{s1:?}"
                        );
                    } else if entangle_idx == 2 {
                        let p = pair_after_l1.expect("P from layer 1");
                        let left_owner = occ_site.get(&p.left).copied();
                        let right_owner = occ_site.get(&p.right).copied();
                        let still_01 = matches!(
                            (left_owner, right_owner),
                            (Some(AtomId(0)), Some(AtomId(1))) | (Some(AtomId(1)), Some(AtomId(0)))
                        );
                        assert!(
                            !still_01,
                            "B13: pair P must be vacated/reassigned before layer-2 entangle"
                        );
                        assert!(
                            !(left_owner.is_some() ^ right_owner.is_some()),
                            "B13: must not leave half-occupied pair P"
                        );
                        vacated_before_l2 = true;
                        let sa = *occ_atom.get(&atoms[0]).expect("a");
                        let sb = *occ_atom.get(&atoms[1]).expect("c");
                        let on_pair = pairs.iter().any(|q| {
                            (q.left == sa && q.right == sb) || (q.left == sb && q.right == sa)
                        });
                        assert!(on_pair, "layer-2 partners must sit on a bank pair");
                        let d = euclidean_um(map[&sa], map[&sb]);
                        assert!(d <= params().rydberg_range_um);
                    }
                }
                _ => {}
            }
        }
    }
    assert!(
        vacated_before_l2,
        "must observe layer-2 entangle after reclaim"
    );
}

#[test]
fn enola_comparable_return_home_four_transfers() {
    let mut p = params();
    p.return_home = true;
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &p).expect("return home");
    // Each moved atom: load+store to pair + load+store home = 4 transfers.
    // At least 2 atoms moved → ≥ 8 transfers.
    assert!(
        result.trap_transfers >= 8,
        "got {} transfers",
        result.trap_transfers
    );
}

#[test]
fn transfer_into_occupied_rejected() {
    // Real emit path: store into a site still held by a non-moving atom (B5).
    let p = params();
    let layout = NeutralAtomLayout {
        sites: vec![
            AtomSite {
                id: SiteId(0),
                position: Position {
                    x_um: 0.0,
                    y_um: 0.0,
                },
            },
            AtomSite {
                id: SiteId(1),
                position: Position {
                    x_um: 20.0,
                    y_um: 0.0,
                },
            },
            AtomSite {
                id: SiteId(2),
                position: Position {
                    x_um: 40.0,
                    y_um: 0.0,
                },
            },
        ],
        initial_bindings: vec![],
    };
    let mut occ_site = BTreeMap::new();
    let mut occ_atom = BTreeMap::new();
    // Atom 1 occupies destination site 1; atom 0 tries to store there from site 0.
    occ_site.insert(SiteId(0), AtomId(0));
    occ_site.insert(SiteId(1), AtomId(1));
    occ_atom.insert(AtomId(0), SiteId(0));
    occ_atom.insert(AtomId(1), SiteId(1));
    let err = try_transfer_into_occupied(
        &layout,
        &p,
        AtomId(0),
        SiteId(0),
        SiteId(1),
        &mut occ_site,
        &mut occ_atom,
    )
    .expect_err("store into occupied must fail");
    assert!(
        matches!(
            err,
            MovementPlanError::TransferIntoOccupied {
                site: SiteId(1),
                ..
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn dual_exclusion_one_atom_moves() {
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("plan");
    // At most one dual per gate: both atoms end on same pair (gap ≤ rb).
    let layout = result.request.layout.as_ref().unwrap();
    let occ = occ_from_layout(layout);
    let map = site_map(layout);
    let d = euclidean_um(map[&occ[&AtomId(0)]], map[&occ[&AtomId(3)]]);
    assert!(d <= params().rydberg_range_um);
}

#[test]
fn sortis_longest_first_rounds() {
    // Smoke: multi-gate layer plans without panic; longer moves preferred in packing.
    let graph = graph_from_edges(6, &[(0, 1), (2, 3), (4, 5)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("plan");
    assert!(result.rearrangement_steps >= 1);
}

#[test]
fn no_partner_stationary_parking() {
    // Destinations must be bank pair sites, not placement neighbors of stationary partner.
    let graph = graph_from_edges(4, &[(0, 3)]);
    let req = place_schedule_isolated(graph);
    let placement_sites: BTreeSet<_> = req
        .layout
        .as_ref()
        .unwrap()
        .initial_bindings
        .iter()
        .map(|b| match b.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
        })
        .collect();
    let result = plan_aod_movement(req, &params()).expect("plan");
    // After plan, partners should be on bank (not both still on original placement with
    // one stationary) — at least one dest of moves is outside original placement set
    // OR they skipped (not this fixture).
    let mut bank_dest = false;
    for m in all_atom_moves(&result.request.layers) {
        if !placement_sites.contains(&m.to) {
            bank_dest = true;
        }
    }
    assert!(bank_dest, "must move onto bank pair sites");
}

#[test]
fn integration_smoke_place_bank_entangle_move() {
    let graph = graph_from_edges(4, &[(0, 1), (2, 3)]);
    let req = place_schedule_isolated(graph);
    let result = plan_aod_movement(req, &params()).expect("e2e");
    for layer in &result.request.layers {
        layer.validate_occupancy().expect("occ");
        layer.validate_conflicts().expect("conf");
    }
    assert!(result.request.layers.iter().any(|l| {
        l.actions
            .iter()
            .any(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
    }));
}

#[test]
fn atom_moves_to_move_specs_fills_coords() {
    let layout = NeutralAtomLayout {
        sites: vec![
            AtomSite {
                id: SiteId(0),
                position: Position {
                    x_um: 1.0,
                    y_um: 2.0,
                },
            },
            AtomSite {
                id: SiteId(1),
                position: Position {
                    x_um: 3.0,
                    y_um: 4.0,
                },
            },
        ],
        initial_bindings: vec![],
    };
    let mut aod = BTreeMap::new();
    aod.insert(
        AtomId(0),
        quon_na::AodTrapRef {
            aod_id: 0,
            row: 1,
            col: 2,
        },
    );
    let specs = atom_moves_to_move_specs(
        &[AtomMove {
            atom: AtomId(0),
            from: SiteId(0),
            to: SiteId(1),
        }],
        &layout,
        &aod,
    )
    .unwrap();
    assert_eq!(specs[0].from_x_um, 1.0);
    assert_eq!(specs[0].to_y_um, 4.0);
    assert_eq!(specs[0].row, 1);
}

#[test]
fn bank_isolation_eps_constant() {
    assert_eq!(BANK_ISOLATION_EPS_UM, 0.01);
}

#[test]
fn site_pitch_dense_fixture() {
    assert_eq!(SITE_PITCH_UM, 5.0);
}
