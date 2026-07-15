//! Fixed-layout three-patch lattice-surgery CX (ADR-0019 / #250).
//!
//! Horsman-style gadget with an **L-shaped** canonical layout so boundaries match
//! surface-code geometry:
//!
//! ```text
//!   Control | Ancilla
//!             --------
//!             Target
//! ```
//!
//! - Rough (ZZ) merge on the shared **left/right** edge (control↔ancilla)
//! - Smooth (XX) merge on the shared **top/bottom** edge (ancilla↔target)
//!
//! Phase sequence:
//! 1. Prepare ancilla in |+⟩ (X-init construct)
//! 2. Rough merge (joint ZZ) control↔ancilla via vertical seam checks
//! 3. Split: re-measure rough seam (projects patches apart)
//! 4. Smooth merge (joint XX) ancilla↔target via horizontal seam checks
//! 5. Split: re-measure smooth seam
//! 6. Measure ancilla logical Z (top-row data product)
//! 7. Record outcome-conditioned Pauli frame updates (no online decoder)
//!
//! Byproducts (Horsman): apply when the named measurement parity is −1
//! (odd record parity): rough → X on target; smooth → Z on control;
//! ancilla Z → Z on control and X on target.
//!
//! Split rounds emit real seam re-measurements (not Wait-only placeholders);
//! surrounding `memory_round` ops restore full patch EC. This is still a
//! simplified hybrid-NA merge–split model — not a general patch router and not
//! a Stim FT-distance claim.

use crate::expand::{
    ExpandError, ExpandedBlock, MergeBoundary, PauliFrameUpdate, PhysicalAtomId, PhysicalCnot,
    PhysicalRound, RoundKind, RoundLocalOp, RoundTerminal, StabilizerDef,
};
use crate::family::{CodeFamily, SourceFamily};
use crate::workload::{LogicalBasis, LogicalQubitId, WorkloadBlock};

/// Gap (lattice units) between adjacent patches, leaving room for a seam of
/// check atoms between facing data rows/columns.
pub const PATCH_GAP: i32 = 2;

/// Expand `logical_cx(control, target)` into merge/split phases + ancilla patch.
///
/// Appends the transitional ancilla [`ExpandedBlock`] to `layouts` and pushes
/// the fixed phase sequence onto `rounds`. Control/target/ancilla coordinates
/// are rewritten into the canonical L-shaped layout.
pub fn expand_logical_cx(
    control: LogicalQubitId,
    target: LogicalQubitId,
    layouts: &mut Vec<ExpandedBlock>,
    next_atom: &mut u32,
    rounds: &mut Vec<PhysicalRound>,
) -> Result<(), ExpandError> {
    let control_idx = find_layout_index(layouts, control)?;
    let target_idx = find_layout_index(layouts, target)?;
    if layouts[control_idx].family != SourceFamily::Surface {
        return Err(ExpandError::LogicalCxNotSurface {
            id: control.0,
            family: layouts[control_idx].family.as_str(),
        });
    }
    if layouts[target_idx].family != SourceFamily::Surface {
        return Err(ExpandError::LogicalCxNotSurface {
            id: target.0,
            family: layouts[target_idx].family.as_str(),
        });
    }
    let distance = layouts[control_idx].distance;
    if layouts[target_idx].distance != distance {
        return Err(ExpandError::LogicalCxDistanceMismatch {
            control_distance: distance,
            target_distance: layouts[target_idx].distance,
        });
    }

    let ancilla_id = next_ancilla_logical_id(layouts);
    let mut ancilla = allocate_ancilla_patch(ancilla_id, distance, next_atom)?;
    let mut seam_ca = allocate_seam_atoms(distance, next_atom)?;
    let mut seam_at = allocate_seam_atoms(distance, next_atom)?;

    // L-shaped place: control | ancilla
    //                         target (below ancilla)
    let d = distance as i32;
    let patch_span = 2 * d + PATCH_GAP;
    place_patch_at(&mut layouts[control_idx], 0, 0);
    place_seam_column(&mut seam_ca, 2 * d + 1, &layouts[control_idx]);
    place_patch_at(&mut ancilla, patch_span, 0);
    place_seam_row(&mut seam_at, 2 * d + 1, &ancilla);
    place_patch_at(&mut layouts[target_idx], patch_span, patch_span);

    let control_layout = layouts[control_idx].clone();
    let target_layout = layouts[target_idx].clone();

    let rough_left = right_column_data(&control_layout)?;
    let rough_right = left_column_data(&ancilla)?;
    let smooth_above = bottom_row_data(&ancilla)?;
    let smooth_below = top_row_data(&target_layout)?;
    let ancilla_logical_z = top_row_data(&ancilla)?;

    // 1. Ancilla |+⟩ prep
    rounds.push(PhysicalRound {
        kind: RoundKind::Construct,
        logical_id: ancilla_id,
        local_before: ancilla
            .data_atoms
            .iter()
            .map(|&atom| RoundLocalOp::H { atom })
            .collect(),
        entangling: Vec::new(),
        z_cnot_count: 0,
        local_mid: Vec::new(),
        local_after: Vec::new(),
        terminal: Vec::new(),
        partner_logical_id: None,
        frame_updates: Vec::new(),
    });

    // 2. Rough merge C↔A (ZZ on L/R seam)
    rounds.push(rough_merge_round(
        &rough_left,
        &rough_right,
        &seam_ca,
        control,
        ancilla_id,
    )?);

    // 3. Split after rough — re-measure the seam (projects patches apart).
    // Full per-patch EC restore is the surrounding memory_round ops, not
    // duplicated here (keeps hybrid NA scheduling tractable).
    rounds.push(split_seam_round(
        MergeBoundary::Rough,
        &rough_left,
        &rough_right,
        &seam_ca,
        control,
        Some(ancilla_id),
    )?);

    // 4. Smooth merge A↔T (XX on top/bottom seam)
    rounds.push(smooth_merge_round(
        &smooth_above,
        &smooth_below,
        &seam_at,
        ancilla_id,
        target,
    )?);

    // 5. Split after smooth — re-measure the smooth seam
    rounds.push(split_seam_round(
        MergeBoundary::Smooth,
        &smooth_above,
        &smooth_below,
        &seam_at,
        ancilla_id,
        Some(target),
    )?);

    // 6. Measure ancilla logical Z (top-row product — not all data)
    rounds.push(PhysicalRound {
        kind: RoundKind::MeasureAncilla,
        logical_id: ancilla_id,
        local_before: Vec::new(),
        entangling: Vec::new(),
        z_cnot_count: 0,
        local_mid: Vec::new(),
        local_after: Vec::new(),
        terminal: ancilla_logical_z
            .iter()
            .map(|&atom| RoundTerminal::Measure {
                atom,
                basis: LogicalBasis::Z,
            })
            .collect(),
        partner_logical_id: None,
        frame_updates: Vec::new(),
    });

    // 7. Outcome-conditioned Pauli frame byproducts (Horsman CX).
    rounds.push(PhysicalRound {
        kind: RoundKind::FrameUpdate,
        logical_id: control,
        local_before: Vec::new(),
        entangling: Vec::new(),
        z_cnot_count: 0,
        local_mid: Vec::new(),
        local_after: Vec::new(),
        terminal: Vec::new(),
        partner_logical_id: Some(target),
        frame_updates: vec![
            PauliFrameUpdate {
                logical_id: target,
                x: true,
                z: false,
                source: "rough_merge",
                condition_atoms: seam_ca.atoms.clone(),
            },
            PauliFrameUpdate {
                logical_id: control,
                x: false,
                z: true,
                source: "smooth_merge",
                condition_atoms: seam_at.atoms.clone(),
            },
            PauliFrameUpdate {
                logical_id: control,
                x: false,
                z: true,
                source: "ancilla_mz",
                condition_atoms: ancilla_logical_z.clone(),
            },
            PauliFrameUpdate {
                logical_id: target,
                x: true,
                z: false,
                source: "ancilla_mz",
                condition_atoms: ancilla_logical_z,
            },
        ],
    });

    // Attach seam atoms + StabilizerDefs to ancilla for atom accounting / check graph.
    for i in 0..seam_ca.atoms.len() {
        let atom = seam_ca.atoms[i];
        ancilla.atoms.push(atom);
        ancilla.coords.push(seam_ca.coords[i]);
        ancilla.check_atoms.push(atom);
        ancilla.stabilizers.push(StabilizerDef {
            check: atom,
            basis: LogicalBasis::Z,
            data: vec![rough_left[i], rough_right[i]],
        });
    }
    for i in 0..seam_at.atoms.len() {
        let atom = seam_at.atoms[i];
        ancilla.atoms.push(atom);
        ancilla.coords.push(seam_at.coords[i]);
        ancilla.check_atoms.push(atom);
        ancilla.stabilizers.push(StabilizerDef {
            check: atom,
            basis: LogicalBasis::X,
            data: vec![smooth_above[i], smooth_below[i]],
        });
    }

    layouts.push(ancilla);
    Ok(())
}

struct SeamAtoms {
    atoms: Vec<PhysicalAtomId>,
    coords: Vec<(i32, i32)>,
}

fn allocate_seam_atoms(distance: u32, next_atom: &mut u32) -> Result<SeamAtoms, ExpandError> {
    let first = *next_atom;
    let last = next_atom
        .checked_add(distance)
        .ok_or(ExpandError::AtomIdOverflow)?;
    *next_atom = last;
    Ok(SeamAtoms {
        atoms: (first..last).map(PhysicalAtomId).collect(),
        coords: vec![(0, 0); distance as usize],
    })
}

fn place_seam_column(seam: &mut SeamAtoms, x: i32, left_patch: &ExpandedBlock) {
    let d = left_patch.distance as usize;
    for r in 0..d {
        seam.coords[r] = (x, 2 * r as i32 + 1);
    }
}

fn place_seam_row(seam: &mut SeamAtoms, y: i32, above_patch: &ExpandedBlock) {
    let d = above_patch.distance as usize;
    // Surface data are a prefix of `atoms`/`coords`; x = origin + 2c + 1.
    let origin_x = above_patch.coords.first().map(|(x, _)| *x - 1).unwrap_or(0);
    for c in 0..d {
        seam.coords[c] = (origin_x + 2 * c as i32 + 1, y);
    }
}

fn allocate_ancilla_patch(
    logical_id: LogicalQubitId,
    distance: u32,
    next_atom: &mut u32,
) -> Result<ExpandedBlock, ExpandError> {
    let meta = WorkloadBlock {
        logical_id,
        family: SourceFamily::Surface,
        distance,
        init_basis: LogicalBasis::X,
        code_family: CodeFamily::SurfaceCodeLike { distance },
    };
    crate::expand::expand_surface_layout_for_surgery(&meta, next_atom)
}

fn next_ancilla_logical_id(layouts: &[ExpandedBlock]) -> LogicalQubitId {
    let max = layouts.iter().map(|b| b.logical_id.0).max().unwrap_or(0);
    LogicalQubitId(max.saturating_add(1))
}

fn find_layout_index(layouts: &[ExpandedBlock], id: LogicalQubitId) -> Result<usize, ExpandError> {
    layouts
        .iter()
        .position(|b| b.logical_id == id)
        .ok_or(ExpandError::UnknownLogicalId(id.0))
}

fn place_patch_at(block: &mut ExpandedBlock, dx: i32, dy: i32) {
    for (x, y) in &mut block.coords {
        *x += dx;
        *y += dy;
    }
}

fn right_column_data(block: &ExpandedBlock) -> Result<Vec<PhysicalAtomId>, ExpandError> {
    let d = block.distance as usize;
    if block.data_atoms.len() != d * d {
        return Err(ExpandError::InvalidPatchData {
            distance: block.distance,
        });
    }
    Ok((0..d).map(|r| block.data_atoms[r * d + (d - 1)]).collect())
}

fn left_column_data(block: &ExpandedBlock) -> Result<Vec<PhysicalAtomId>, ExpandError> {
    let d = block.distance as usize;
    if block.data_atoms.len() != d * d {
        return Err(ExpandError::InvalidPatchData {
            distance: block.distance,
        });
    }
    Ok((0..d).map(|r| block.data_atoms[r * d]).collect())
}

fn top_row_data(block: &ExpandedBlock) -> Result<Vec<PhysicalAtomId>, ExpandError> {
    let d = block.distance as usize;
    if block.data_atoms.len() != d * d {
        return Err(ExpandError::InvalidPatchData {
            distance: block.distance,
        });
    }
    Ok(block.data_atoms[..d].to_vec())
}

fn bottom_row_data(block: &ExpandedBlock) -> Result<Vec<PhysicalAtomId>, ExpandError> {
    let d = block.distance as usize;
    if block.data_atoms.len() != d * d {
        return Err(ExpandError::InvalidPatchData {
            distance: block.distance,
        });
    }
    Ok(block.data_atoms[(d - 1) * d..d * d].to_vec())
}

/// Rough merge: measure ZZ on each facing L/R data pair via seam check.
fn rough_merge_round(
    left_col: &[PhysicalAtomId],
    right_col: &[PhysicalAtomId],
    seam: &SeamAtoms,
    primary: LogicalQubitId,
    partner: LogicalQubitId,
) -> Result<PhysicalRound, ExpandError> {
    debug_assert_eq!(left_col.len(), seam.atoms.len());
    debug_assert_eq!(right_col.len(), seam.atoms.len());
    let mut entangling = Vec::with_capacity(2 * seam.atoms.len());
    for i in 0..seam.atoms.len() {
        entangling.push(PhysicalCnot {
            control: left_col[i],
            target: seam.atoms[i],
        });
        entangling.push(PhysicalCnot {
            control: right_col[i],
            target: seam.atoms[i],
        });
    }
    let z_cnot_count = entangling.len();
    let mut terminal = Vec::with_capacity(2 * seam.atoms.len());
    for &atom in &seam.atoms {
        terminal.push(RoundTerminal::Measure {
            atom,
            basis: LogicalBasis::Z,
        });
    }
    for &atom in &seam.atoms {
        terminal.push(RoundTerminal::Reset { atom });
    }
    Ok(PhysicalRound {
        kind: RoundKind::Merge(MergeBoundary::Rough),
        logical_id: primary,
        local_before: Vec::new(),
        entangling,
        z_cnot_count,
        local_mid: Vec::new(),
        local_after: Vec::new(),
        terminal,
        partner_logical_id: Some(partner),
        frame_updates: Vec::new(),
    })
}

/// Smooth merge: measure XX on each facing top/bottom data pair (H-sandwich).
fn smooth_merge_round(
    above_row: &[PhysicalAtomId],
    below_row: &[PhysicalAtomId],
    seam: &SeamAtoms,
    primary: LogicalQubitId,
    partner: LogicalQubitId,
) -> Result<PhysicalRound, ExpandError> {
    debug_assert_eq!(above_row.len(), seam.atoms.len());
    debug_assert_eq!(below_row.len(), seam.atoms.len());
    let mut local_mid = Vec::with_capacity(seam.atoms.len());
    let mut x_cnots = Vec::with_capacity(2 * seam.atoms.len());
    for i in 0..seam.atoms.len() {
        local_mid.push(RoundLocalOp::H {
            atom: seam.atoms[i],
        });
        x_cnots.push(PhysicalCnot {
            control: seam.atoms[i],
            target: above_row[i],
        });
        x_cnots.push(PhysicalCnot {
            control: seam.atoms[i],
            target: below_row[i],
        });
    }
    let mut terminal = Vec::with_capacity(2 * seam.atoms.len());
    for &atom in &seam.atoms {
        terminal.push(RoundTerminal::Measure {
            atom,
            basis: LogicalBasis::Z,
        });
    }
    for &atom in &seam.atoms {
        terminal.push(RoundTerminal::Reset { atom });
    }
    Ok(PhysicalRound {
        kind: RoundKind::Merge(MergeBoundary::Smooth),
        logical_id: primary,
        local_before: Vec::new(),
        entangling: x_cnots,
        z_cnot_count: 0,
        local_mid: local_mid.clone(),
        local_after: local_mid,
        terminal,
        partner_logical_id: Some(partner),
        frame_updates: Vec::new(),
    })
}

/// Split: re-measure the seam to project patches apart (physical ops, not Wait).
///
/// Rough seams re-measure ZZ pairs; smooth seams re-measure XX (H-sandwich).
/// Surrounding `memory_round` ops restore full patch stabilizer structure.
fn split_seam_round(
    boundary: MergeBoundary,
    side_a: &[PhysicalAtomId],
    side_b: &[PhysicalAtomId],
    seam: &SeamAtoms,
    primary: LogicalQubitId,
    partner: Option<LogicalQubitId>,
) -> Result<PhysicalRound, ExpandError> {
    let mut round = match boundary {
        MergeBoundary::Rough => {
            rough_merge_round(side_a, side_b, seam, primary, partner.unwrap_or(primary))?
        }
        MergeBoundary::Smooth => {
            smooth_merge_round(side_a, side_b, seam, primary, partner.unwrap_or(primary))?
        }
    };
    round.kind = RoundKind::Split(boundary);
    round.partner_logical_id = partner;
    Ok(round)
}
