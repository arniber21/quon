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

    // 6. Measure ancilla logical Z (top-row product — not all data), then
    // reset the whole ancilla footprint. The gadget leaves the ancilla fully
    // disentangled from control/target at this point (byproducts corrected
    // via the frame updates below), so every ancilla atom — not just the
    // measured top row — is safe to reset; without this the un-reset
    // bulk/check atoms stay physically "in the way" and trip the dialect
    // verifier the first time a later op has to move them.
    let mut ancilla_mz_terminal: Vec<RoundTerminal> = ancilla_logical_z
        .iter()
        .map(|&atom| RoundTerminal::Measure {
            atom,
            basis: LogicalBasis::Z,
        })
        .collect();
    ancilla_mz_terminal.extend(
        ancilla
            .data_atoms
            .iter()
            .chain(ancilla.check_atoms.iter())
            .map(|&atom| RoundTerminal::Reset { atom }),
    );
    rounds.push(PhysicalRound {
        kind: RoundKind::MeasureAncilla,
        logical_id: ancilla_id,
        local_before: Vec::new(),
        entangling: Vec::new(),
        z_cnot_count: 0,
        local_mid: Vec::new(),
        local_after: Vec::new(),
        terminal: ancilla_mz_terminal,
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

pub(crate) struct SeamAtoms {
    pub(crate) atoms: Vec<PhysicalAtomId>,
    pub(crate) coords: Vec<(i32, i32)>,
}

pub(crate) fn allocate_seam_atoms(
    distance: u32,
    next_atom: &mut u32,
) -> Result<SeamAtoms, ExpandError> {
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

pub(crate) fn place_seam_column(seam: &mut SeamAtoms, x: i32, left_patch: &ExpandedBlock) {
    let d = left_patch.distance as usize;
    for r in 0..d {
        seam.coords[r] = (x, 2 * r as i32 + 1);
    }
}

pub(crate) fn place_seam_row(seam: &mut SeamAtoms, y: i32, above_patch: &ExpandedBlock) {
    let d = above_patch.distance as usize;
    // Surface data are a prefix of `atoms`/`coords`; x = origin + 2c + 1.
    let origin_x = above_patch.coords.first().map(|(x, _)| *x - 1).unwrap_or(0);
    for c in 0..d {
        seam.coords[c] = (origin_x + 2 * c as i32 + 1, y);
    }
}

pub(crate) fn allocate_ancilla_patch(
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

pub(crate) fn next_ancilla_logical_id(layouts: &[ExpandedBlock]) -> LogicalQubitId {
    let max = layouts.iter().map(|b| b.logical_id.0).max().unwrap_or(0);
    LogicalQubitId(max.saturating_add(1))
}

pub(crate) fn find_layout_index(
    layouts: &[ExpandedBlock],
    id: LogicalQubitId,
) -> Result<usize, ExpandError> {
    layouts
        .iter()
        .position(|b| b.logical_id == id)
        .ok_or(ExpandError::UnknownLogicalId(id.0))
}

pub(crate) fn place_patch_at(block: &mut ExpandedBlock, dx: i32, dy: i32) {
    for (x, y) in &mut block.coords {
        *x += dx;
        *y += dy;
    }
}

// Flux verifies this without `#[trusted]`: the `d == 0` guard refines `d >= 1`
// (so `d - 1` cannot underflow) and `len == d * d` lets flux-infer prove
// `r * d + (d - 1) < d * d` for every `r in 0..d` — the same nonlinear
// multiplication reasoning it already applies to `left_column_data`'s
// `r * d` index. Direct indexing (no `chunks`) keeps the bound a single
// `Vec::index` obligation flux-infer can discharge.
pub(crate) fn right_column_data(block: &ExpandedBlock) -> Result<Vec<PhysicalAtomId>, ExpandError> {
    let d = block.distance as usize;
    if d == 0 || block.data_atoms.len() != d * d {
        return Err(ExpandError::InvalidPatchData {
            distance: block.distance,
        });
    }
    Ok((0..d)
        .map(|r| block.data_atoms[r * d + (d - 1)])
        .collect())
}

pub(crate) fn left_column_data(block: &ExpandedBlock) -> Result<Vec<PhysicalAtomId>, ExpandError> {
    let d = block.distance as usize;
    if block.data_atoms.len() != d * d {
        return Err(ExpandError::InvalidPatchData {
            distance: block.distance,
        });
    }
    Ok((0..d).map(|r| block.data_atoms[r * d]).collect())
}

pub(crate) fn top_row_data(block: &ExpandedBlock) -> Result<Vec<PhysicalAtomId>, ExpandError> {
    let d = block.distance as usize;
    if block.data_atoms.len() != d * d {
        return Err(ExpandError::InvalidPatchData {
            distance: block.distance,
        });
    }
    Ok(block.data_atoms[..d].to_vec())
}

// Flux verifies this without `#[trusted]`: the `d == 0` guard refines `d >= 1`.
// The explicit `len < d` guard below lets flux-infer derive `d <= len` by a
// direct branch comparison (no nonlinear arithmetic needed), so `len - d`
// cannot underflow and the `len - d..` slice start is in bounds. This avoids
// the `d*d >= d` nonlinear reasoning flux-infer cannot perform.
pub(crate) fn bottom_row_data(block: &ExpandedBlock) -> Result<Vec<PhysicalAtomId>, ExpandError> {
    let d = block.distance as usize;
    if d == 0 || block.data_atoms.len() != d * d {
        return Err(ExpandError::InvalidPatchData {
            distance: block.distance,
        });
    }
    let len = block.data_atoms.len();
    // Explicit guard so Flux derives `d <= len` by direct comparison (not the
    // nonlinear `d*d >= d`): makes `len - d` and the slice start verifiable.
    if len < d {
        return Err(ExpandError::InvalidPatchData {
            distance: block.distance,
        });
    }
    // Last `d` elements.
    Ok(block.data_atoms[len - d..].to_vec())
}

/// Rough merge: measure ZZ on each facing L/R data pair via seam check.
/// Flux verifies this without `#[trusted]`: the visible `left_col.len() == n`
/// / `right_col.len() == n` guards (where `n = seam.atoms.len()`) let
/// flux-infer prove `left_col[i]`, `right_col[i]`, and `seam.atoms[i]` are in
/// bounds for every `i < n` in the loop body.
pub(crate) fn rough_merge_round(
    left_col: &[PhysicalAtomId],
    right_col: &[PhysicalAtomId],
    seam: &SeamAtoms,
    primary: LogicalQubitId,
    partner: LogicalQubitId,
) -> Result<PhysicalRound, ExpandError> {
    let n = seam.atoms.len();
    // Visible length checks so flux can see all three slices share length `n`.
    if left_col.len() != n || right_col.len() != n {
        return Err(ExpandError::SeamLengthMismatch {
            left: left_col.len(),
            right: right_col.len(),
            seam: n,
        });
    }
    let mut entangling = Vec::with_capacity(2 * n);
    // Iterator-zip iteration so Flux proves bounds through the shared length
    // (all three slices have length `n`) rather than projecting an index
    // refinement through the loop body — the closure-projection weakness of
    // ADR-0027 does not affect `Iterator::zip`.
    for ((&l, &r), &s) in left_col
        .iter()
        .zip(right_col.iter())
        .zip(&seam.atoms)
    {
        entangling.push(PhysicalCnot {
            control: l,
            target: s,
        });
        entangling.push(PhysicalCnot {
            control: r,
            target: s,
        });
    }
    let z_cnot_count = entangling.len();
    let mut terminal = Vec::with_capacity(2 * n);
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
/// Flux verifies this without `#[trusted]`: the visible `above_row.len() == n`
/// / `below_row.len() == n` guards (where `n = seam.atoms.len()`) plus
/// iterator-zip iteration let flux-infer prove every accessed element is in
/// bounds — the closure-projection weakness of ADR-0027 does not affect
/// `Iterator::zip`.
pub(crate) fn smooth_merge_round(
    above_row: &[PhysicalAtomId],
    below_row: &[PhysicalAtomId],
    seam: &SeamAtoms,
    primary: LogicalQubitId,
    partner: LogicalQubitId,
) -> Result<PhysicalRound, ExpandError> {
    let n = seam.atoms.len();
    // Visible length checks so flux can see all three slices share length `n`.
    if above_row.len() != n || below_row.len() != n {
        return Err(ExpandError::SeamLengthMismatch {
            left: above_row.len(),
            right: below_row.len(),
            seam: n,
        });
    }
    let mut local_mid = Vec::with_capacity(n);
    let mut x_cnots = Vec::with_capacity(2 * n);
    // Iterator-zip iteration so Flux proves bounds through the shared length
    // rather than projecting an index refinement through the loop body.
    for (&s, (&a, &b)) in seam.atoms.iter().zip(above_row.iter().zip(below_row.iter())) {
        local_mid.push(RoundLocalOp::H { atom: s });
        x_cnots.push(PhysicalCnot {
            control: s,
            target: a,
        });
        x_cnots.push(PhysicalCnot {
            control: s,
            target: b,
        });
    }
    let mut terminal = Vec::with_capacity(2 * n);
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
pub(crate) fn split_seam_round(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expand::ExpandError;

    /// Build a surface-code `ExpandedBlock` with `d²` data atoms in row-major
    /// order (atom id = `r * d + c`), matching `expand_surface_layout`.
    fn surface_block(distance: u32) -> ExpandedBlock {
        let d = distance as usize;
        let data_atoms: Vec<PhysicalAtomId> =
            (0..d * d).map(|i| PhysicalAtomId(i as u32)).collect();
        ExpandedBlock {
            logical_id: LogicalQubitId(0),
            family: SourceFamily::Surface,
            code_family: CodeFamily::SurfaceCodeLike { distance },
            distance,
            init_basis: LogicalBasis::Z,
            atoms: data_atoms.clone(),
            data_atoms,
            check_atoms: Vec::new(),
            coords: Vec::new(),
            stabilizers: Vec::new(),
        }
    }

    /// Build a block with a custom `data_atoms` length (for malformed-patch tests).
    fn block_with_data(distance: u32, data_len: usize) -> ExpandedBlock {
        let mut block = surface_block(distance);
        block.data_atoms = (0..data_len).map(|i| PhysicalAtomId(i as u32)).collect();
        block
    }

    fn seam_with_n(n: usize) -> SeamAtoms {
        SeamAtoms {
            atoms: (0..n as u32).map(PhysicalAtomId).collect(),
            coords: vec![(0, 0); n],
        }
    }

    // --- right_column_data ---

    #[test]
    fn right_column_distance_zero_rejected() {
        let block = surface_block(0);
        assert!(matches!(
            right_column_data(&block),
            Err(ExpandError::InvalidPatchData { distance: 0 })
        ));
    }

    #[test]
    fn right_column_malformed_patch_rejected() {
        // distance 3 but only 8 data atoms (not 9)
        let block = block_with_data(3, 8);
        assert!(matches!(
            right_column_data(&block),
            Err(ExpandError::InvalidPatchData { distance: 3 })
        ));
    }

    #[test]
    fn right_column_valid_d3() {
        let block = surface_block(3);
        let col = right_column_data(&block).expect("d3");
        // Row-major: right column = indices 2, 5, 8
        assert_eq!(col, vec![PhysicalAtomId(2), PhysicalAtomId(5), PhysicalAtomId(8)]);
    }

    #[test]
    fn right_column_valid_d5() {
        let block = surface_block(5);
        let col = right_column_data(&block).expect("d5");
        // Right column = indices 4, 9, 14, 19, 24
        assert_eq!(
            col,
            vec![
                PhysicalAtomId(4),
                PhysicalAtomId(9),
                PhysicalAtomId(14),
                PhysicalAtomId(19),
                PhysicalAtomId(24),
            ]
        );
    }

    // --- bottom_row_data ---

    #[test]
    fn bottom_row_distance_zero_rejected() {
        let block = surface_block(0);
        assert!(matches!(
            bottom_row_data(&block),
            Err(ExpandError::InvalidPatchData { distance: 0 })
        ));
    }

    #[test]
    fn bottom_row_malformed_patch_rejected() {
        // distance 3 but 10 data atoms (not 9)
        let block = block_with_data(3, 10);
        assert!(matches!(
            bottom_row_data(&block),
            Err(ExpandError::InvalidPatchData { distance: 3 })
        ));
    }

    #[test]
    fn bottom_row_valid_d3() {
        let block = surface_block(3);
        let row = bottom_row_data(&block).expect("d3");
        // Last 3 of 9: indices 6, 7, 8
        assert_eq!(row, vec![PhysicalAtomId(6), PhysicalAtomId(7), PhysicalAtomId(8)]);
    }

    #[test]
    fn bottom_row_valid_d5() {
        let block = surface_block(5);
        let row = bottom_row_data(&block).expect("d5");
        // Last 5 of 25: indices 20..24
        assert_eq!(
            row,
            vec![
                PhysicalAtomId(20),
                PhysicalAtomId(21),
                PhysicalAtomId(22),
                PhysicalAtomId(23),
                PhysicalAtomId(24),
            ]
        );
    }

    // --- rough_merge_round ---

    #[test]
    fn rough_merge_mismatched_seam_lengths_rejected() {
        let left = vec![PhysicalAtomId(0), PhysicalAtomId(1), PhysicalAtomId(2)];
        let right = vec![PhysicalAtomId(3), PhysicalAtomId(4)]; // only 2, not 3
        let seam = seam_with_n(3);
        let err = rough_merge_round(
            &left,
            &right,
            &seam,
            LogicalQubitId(0),
            LogicalQubitId(1),
        )
        .unwrap_err();
        assert!(
            matches!(err, ExpandError::SeamLengthMismatch { left: 3, right: 2, seam: 3 }),
            "got {err:?}"
        );
    }

    #[test]
    fn rough_merge_mismatched_left_rejected() {
        let left = vec![PhysicalAtomId(0)]; // only 1, not 3
        let right = vec![PhysicalAtomId(3), PhysicalAtomId(4), PhysicalAtomId(5)];
        let seam = seam_with_n(3);
        assert!(matches!(
            rough_merge_round(&left, &right, &seam, LogicalQubitId(0), LogicalQubitId(1)),
            Err(ExpandError::SeamLengthMismatch { left: 1, right: 3, seam: 3 })
        ));
    }

    #[test]
    fn rough_merge_valid_d3() {
        let left = vec![PhysicalAtomId(0), PhysicalAtomId(1), PhysicalAtomId(2)];
        let right = vec![PhysicalAtomId(3), PhysicalAtomId(4), PhysicalAtomId(5)];
        let seam = seam_with_n(3);
        let round = rough_merge_round(
            &left,
            &right,
            &seam,
            LogicalQubitId(0),
            LogicalQubitId(1),
        )
        .expect("d3 rough merge");

        assert_eq!(round.kind, RoundKind::Merge(MergeBoundary::Rough));
        // 3 pairs × 2 CNOTs each = 6
        assert_eq!(round.entangling.len(), 6);
        assert_eq!(round.z_cnot_count, 6);
        // Each seam atom measured + reset = 2 * 3 = 6 terminals
        assert_eq!(round.terminal.len(), 6);
        assert_eq!(round.partner_logical_id, Some(LogicalQubitId(1)));

        // Verify the pairing: left[i]→seam[i], right[i]→seam[i]
        for i in 0..3 {
            assert_eq!(round.entangling[2 * i].control, left[i]);
            assert_eq!(round.entangling[2 * i].target, seam.atoms[i]);
            assert_eq!(round.entangling[2 * i + 1].control, right[i]);
            assert_eq!(round.entangling[2 * i + 1].target, seam.atoms[i]);
        }
    }

    #[test]
    fn rough_merge_valid_d5() {
        let left: Vec<_> = (0..5).map(PhysicalAtomId).collect();
        let right: Vec<_> = (10..15).map(PhysicalAtomId).collect();
        let seam = seam_with_n(5);
        let round = rough_merge_round(
            &left,
            &right,
            &seam,
            LogicalQubitId(0),
            LogicalQubitId(1),
        )
        .expect("d5 rough merge");

        assert_eq!(round.entangling.len(), 10); // 5 pairs × 2
        assert_eq!(round.z_cnot_count, 10);
        assert_eq!(round.terminal.len(), 10); // 5 measures + 5 resets
    }

    #[test]
    fn rough_merge_empty_seam_ok() {
        // n=0: no CNOTs, no terminals — degenerate but safe.
        let round = rough_merge_round(
            &[],
            &[],
            &seam_with_n(0),
            LogicalQubitId(0),
            LogicalQubitId(1),
        )
        .expect("empty rough merge");
        assert!(round.entangling.is_empty());
        assert!(round.terminal.is_empty());
    }

    // --- smooth_merge_round ---

    #[test]
    fn smooth_merge_mismatched_seam_lengths_rejected() {
        let above = vec![PhysicalAtomId(0), PhysicalAtomId(1), PhysicalAtomId(2)];
        let below = vec![PhysicalAtomId(3), PhysicalAtomId(4)]; // only 2
        let seam = seam_with_n(3);
        let err = smooth_merge_round(
            &above,
            &below,
            &seam,
            LogicalQubitId(0),
            LogicalQubitId(1),
        )
        .unwrap_err();
        assert!(
            matches!(err, ExpandError::SeamLengthMismatch { left: 3, right: 2, seam: 3 }),
            "got {err:?}"
        );
    }

    #[test]
    fn smooth_merge_mismatched_below_rejected() {
        let above = vec![PhysicalAtomId(0), PhysicalAtomId(1)];
        let below = vec![PhysicalAtomId(3), PhysicalAtomId(4), PhysicalAtomId(5)];
        let seam = seam_with_n(3);
        assert!(matches!(
            smooth_merge_round(&above, &below, &seam, LogicalQubitId(0), LogicalQubitId(1)),
            Err(ExpandError::SeamLengthMismatch { left: 2, right: 3, seam: 3 })
        ));
    }

    #[test]
    fn smooth_merge_valid_d3() {
        let above = vec![PhysicalAtomId(0), PhysicalAtomId(1), PhysicalAtomId(2)];
        let below = vec![PhysicalAtomId(3), PhysicalAtomId(4), PhysicalAtomId(5)];
        let seam = seam_with_n(3);
        let round = smooth_merge_round(
            &above,
            &below,
            &seam,
            LogicalQubitId(0),
            LogicalQubitId(1),
        )
        .expect("d3 smooth merge");

        assert_eq!(round.kind, RoundKind::Merge(MergeBoundary::Smooth));
        assert_eq!(round.z_cnot_count, 0);
        // 3 H-gates in local_mid (one per seam atom)
        assert_eq!(round.local_mid.len(), 3);
        // X-sandwich: seam→above and seam→below = 6 CNOTs
        assert_eq!(round.entangling.len(), 6);
        assert_eq!(round.terminal.len(), 6); // 3 measures + 3 resets
        assert_eq!(round.partner_logical_id, Some(LogicalQubitId(1)));

        // Verify pairing: seam[i]→above[i], seam[i]→below[i]
        for i in 0..3 {
            assert_eq!(round.entangling[2 * i].control, seam.atoms[i]);
            assert_eq!(round.entangling[2 * i].target, above[i]);
            assert_eq!(round.entangling[2 * i + 1].control, seam.atoms[i]);
            assert_eq!(round.entangling[2 * i + 1].target, below[i]);
        }
        // local_mid is reused as local_after
        assert_eq!(round.local_after, round.local_mid);
    }

    #[test]
    fn smooth_merge_valid_d5() {
        let above: Vec<_> = (0..5).map(PhysicalAtomId).collect();
        let below: Vec<_> = (10..15).map(PhysicalAtomId).collect();
        let seam = seam_with_n(5);
        let round = smooth_merge_round(
            &above,
            &below,
            &seam,
            LogicalQubitId(0),
            LogicalQubitId(1),
        )
        .expect("d5 smooth merge");

        assert_eq!(round.entangling.len(), 10); // 5 pairs × 2
        assert_eq!(round.local_mid.len(), 5);
        assert_eq!(round.terminal.len(), 10); // 5 measures + 5 resets
    }

    #[test]
    fn smooth_merge_empty_seam_ok() {
        let round = smooth_merge_round(
            &[],
            &[],
            &seam_with_n(0),
            LogicalQubitId(0),
            LogicalQubitId(1),
        )
        .expect("empty smooth merge");
        assert!(round.entangling.is_empty());
        assert!(round.local_mid.is_empty());
        assert!(round.terminal.is_empty());
    }

    // --- split_seam_round delegates correctly ---

    #[test]
    fn split_seam_rough_delegates_to_rough_merge() {
        let left = vec![PhysicalAtomId(0), PhysicalAtomId(1), PhysicalAtomId(2)];
        let right = vec![PhysicalAtomId(3), PhysicalAtomId(4), PhysicalAtomId(5)];
        let seam = seam_with_n(3);
        let round = split_seam_round(
            MergeBoundary::Rough,
            &left,
            &right,
            &seam,
            LogicalQubitId(0),
            Some(LogicalQubitId(1)),
        )
        .expect("split rough");
        assert_eq!(round.kind, RoundKind::Split(MergeBoundary::Rough));
        assert_eq!(round.partner_logical_id, Some(LogicalQubitId(1)));
    }

    #[test]
    fn split_seam_smooth_delegates_to_smooth_merge() {
        let above = vec![PhysicalAtomId(0), PhysicalAtomId(1), PhysicalAtomId(2)];
        let below = vec![PhysicalAtomId(3), PhysicalAtomId(4), PhysicalAtomId(5)];
        let seam = seam_with_n(3);
        let round = split_seam_round(
            MergeBoundary::Smooth,
            &above,
            &below,
            &seam,
            LogicalQubitId(0),
            Some(LogicalQubitId(1)),
        )
        .expect("split smooth");
        assert_eq!(round.kind, RoundKind::Split(MergeBoundary::Smooth));
    }

    #[test]
    fn split_seam_propagates_length_mismatch() {
        let left = vec![PhysicalAtomId(0), PhysicalAtomId(1)];
        let right = vec![PhysicalAtomId(3), PhysicalAtomId(4), PhysicalAtomId(5)];
        let seam = seam_with_n(3);
        assert!(matches!(
            split_seam_round(
                MergeBoundary::Rough,
                &left,
                &right,
                &seam,
                LogicalQubitId(0),
                None,
            ),
            Err(ExpandError::SeamLengthMismatch { .. })
        ));
    }
}
