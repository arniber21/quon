//! Fixed-layout three-patch lattice-surgery CX (ADR-0019 / #250).
//!
//! Horsman-style gadget: control | transitional ancilla | target in a canonical
//! linear layout. Phase sequence is fixed:
//!
//! 1. Prepare ancilla in |+⟩ (X-init construct)
//! 2. Rough merge (joint ZZ) control↔ancilla via seam checks
//! 3. Split (round barrier; patches resume as separate codes)
//! 4. Smooth merge (joint XX) ancilla↔target via seam checks
//! 5. Split
//! 6. Measure ancilla in Z (byproduct)
//! 7. Record Pauli frame updates (no online decoder)
//!
//! Seam checks sit in the one-site gap between adjacent patches. This is a
//! simplified merge–split model for hybrid NA scheduling — not a general patch
//! router and not a claim of Stim-equivalent FT distance.

use crate::expand::{
    ExpandError, ExpandedBlock, PhysicalAtomId, PhysicalCnot, PhysicalRound, RoundKind,
    RoundLocalOp, RoundTerminal, MergeBoundary, PauliFrameUpdate,
};
use crate::family::{CodeFamily, SourceFamily};
use crate::workload::{LogicalBasis, LogicalQubitId, WorkloadBlock};

/// Horizontal gap (lattice units) between adjacent patches in the linear layout.
/// Leaves room for a column of seam-check atoms between facing data columns.
pub const PATCH_GAP: i32 = 2;

/// Expand `logical_cx(control, target)` into merge/split phases + ancilla patch.
///
/// Appends the transitional ancilla [`ExpandedBlock`] to `layouts` and pushes
/// the fixed phase sequence onto `rounds`. Control/target/ancilla coordinates
/// are rewritten into the canonical linear layout.
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

    // Linear place: control | seam_ca | ancilla | seam_at | target
    let d = distance as i32;
    let patch_span = 2 * d + PATCH_GAP; // data extent 2d plus gap before next patch
    place_patch_at(&mut layouts[control_idx], 0);
    place_seam_column(&mut seam_ca, 2 * d + 1, &layouts[control_idx]);
    place_patch_at(&mut ancilla, patch_span);
    place_seam_column(&mut seam_at, patch_span + 2 * d + 1, &ancilla);
    place_patch_at(&mut layouts[target_idx], 2 * patch_span);

    // Re-borrow after mutations.
    let control_layout = layouts[control_idx].clone();
    let target_layout = layouts[target_idx].clone();

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

    // 2. Rough merge C↔A (ZZ seam)
    rounds.push(rough_merge_round(
        &control_layout,
        &ancilla,
        &seam_ca,
        control,
        ancilla_id,
    )?);

    // 3. Split after rough
    rounds.push(split_round(MergeBoundary::Rough, control, Some(ancilla_id)));

    // 4. Smooth merge A↔T (XX seam)
    rounds.push(smooth_merge_round(
        &ancilla,
        &target_layout,
        &seam_at,
        ancilla_id,
        target,
    )?);

    // 5. Split after smooth
    rounds.push(split_round(MergeBoundary::Smooth, ancilla_id, Some(target)));

    // 6. Measure ancilla in Z (byproduct source)
    rounds.push(PhysicalRound {
        kind: RoundKind::MeasureAncilla,
        logical_id: ancilla_id,
        local_before: Vec::new(),
        entangling: Vec::new(),
        z_cnot_count: 0,
        local_mid: Vec::new(),
        local_after: Vec::new(),
        terminal: ancilla
            .data_atoms
            .iter()
            .map(|&atom| RoundTerminal::Measure {
                atom,
                basis: LogicalBasis::Z,
            })
            .collect(),
        partner_logical_id: None,
        frame_updates: Vec::new(),
    });

    // 7. Pauli frame byproducts (Horsman CX): rough merge → X on target;
    //    smooth merge → Z on control; ancilla Z → Z on control & X on target.
    //    Recorded as frame IR / Stim observable bookkeeping — no online decoder.
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
            },
            PauliFrameUpdate {
                logical_id: control,
                x: false,
                z: true,
                source: "smooth_merge",
            },
            PauliFrameUpdate {
                logical_id: control,
                x: false,
                z: true,
                source: "ancilla_mz",
            },
            PauliFrameUpdate {
                logical_id: target,
                x: true,
                z: false,
                source: "ancilla_mz",
            },
        ],
    });

    // Attach seam atoms to ancilla block for atom accounting / Stim coords.
    for (atom, coord) in seam_ca.atoms.iter().zip(seam_ca.coords.iter()) {
        ancilla.atoms.push(*atom);
        ancilla.coords.push(*coord);
        ancilla.check_atoms.push(*atom);
    }
    for (atom, coord) in seam_at.atoms.iter().zip(seam_at.coords.iter()) {
        ancilla.atoms.push(*atom);
        ancilla.coords.push(*coord);
        ancilla.check_atoms.push(*atom);
    }

    layouts.push(ancilla);
    Ok(())
}

struct SeamColumn {
    atoms: Vec<PhysicalAtomId>,
    coords: Vec<(i32, i32)>,
}

fn allocate_seam_atoms(distance: u32, next_atom: &mut u32) -> Result<SeamColumn, ExpandError> {
    let first = *next_atom;
    let last = next_atom
        .checked_add(distance)
        .ok_or(ExpandError::AtomIdOverflow)?;
    *next_atom = last;
    Ok(SeamColumn {
        atoms: (first..last).map(PhysicalAtomId).collect(),
        coords: vec![(0, 0); distance as usize],
    })
}

fn place_seam_column(seam: &mut SeamColumn, x: i32, left_patch: &ExpandedBlock) {
    let d = left_patch.distance as usize;
    for r in 0..d {
        // Align with data row y = 2*r+1
        seam.coords[r] = (x, 2 * r as i32 + 1);
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
    // Reuse surface layout via a thin local copy of the geometry rules by
    // calling through expand's public surface path — implemented inline to
    // avoid circular module deps: duplicate the call via crate::expand helper.
    crate::expand::expand_surface_layout_for_surgery(&meta, next_atom)
}

fn next_ancilla_logical_id(layouts: &[ExpandedBlock]) -> LogicalQubitId {
    let max = layouts
        .iter()
        .map(|b| b.logical_id.0)
        .max()
        .unwrap_or(0);
    LogicalQubitId(max.saturating_add(1))
}

fn find_layout_index(
    layouts: &[ExpandedBlock],
    id: LogicalQubitId,
) -> Result<usize, ExpandError> {
    layouts
        .iter()
        .position(|b| b.logical_id == id)
        .ok_or(ExpandError::UnknownLogicalId(id.0))
}

fn place_patch_at(block: &mut ExpandedBlock, dx: i32) {
    for (x, _) in &mut block.coords {
        *x += dx;
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

/// Rough merge: measure ZZ on each facing data pair via seam check.
fn rough_merge_round(
    left: &ExpandedBlock,
    right: &ExpandedBlock,
    seam: &SeamColumn,
    primary: LogicalQubitId,
    partner: LogicalQubitId,
) -> Result<PhysicalRound, ExpandError> {
    let left_col = right_column_data(left)?;
    let right_col = left_column_data(right)?;
    debug_assert_eq!(left_col.len(), seam.atoms.len());
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

/// Smooth merge: measure XX on each facing data pair (H-sandwich on seam).
fn smooth_merge_round(
    left: &ExpandedBlock,
    right: &ExpandedBlock,
    seam: &SeamColumn,
    primary: LogicalQubitId,
    partner: LogicalQubitId,
) -> Result<PhysicalRound, ExpandError> {
    let left_col = right_column_data(left)?;
    let right_col = left_column_data(right)?;
    let mut local_mid = Vec::with_capacity(seam.atoms.len());
    let mut x_cnots = Vec::with_capacity(2 * seam.atoms.len());
    for i in 0..seam.atoms.len() {
        local_mid.push(RoundLocalOp::H {
            atom: seam.atoms[i],
        });
        // X-check style: CX(check → data)
        x_cnots.push(PhysicalCnot {
            control: seam.atoms[i],
            target: left_col[i],
        });
        x_cnots.push(PhysicalCnot {
            control: seam.atoms[i],
            target: right_col[i],
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
        z_cnot_count: 0, // all CXs are X-phase (after mid H)
        local_mid: local_mid.clone(),
        local_after: local_mid,
        terminal,
        partner_logical_id: Some(partner),
        frame_updates: Vec::new(),
    })
}

fn split_round(
    boundary: MergeBoundary,
    primary: LogicalQubitId,
    partner: Option<LogicalQubitId>,
) -> PhysicalRound {
    PhysicalRound {
        kind: RoundKind::Split(boundary),
        logical_id: primary,
        local_before: Vec::new(),
        entangling: Vec::new(),
        z_cnot_count: 0,
        local_mid: Vec::new(),
        local_after: Vec::new(),
        terminal: Vec::new(),
        partner_logical_id: partner,
        frame_updates: Vec::new(),
    }
}
