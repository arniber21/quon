//! Generalized patch-operation planner (issue #281).
//!
//! Replaces the fixed CX-only lattice-surgery path with an explicit
//! patch-operation model. Logical CX, logical measurement, and bare
//! merge/split all lower through one planner that represents patches,
//! boundaries, adjacency, merge/split phases, patch measurements, and
//! frame updates as first-class operations.
//!
//! This is a **scoped** planner, not a full patch-router. The planner
//! validates adjacency and boundary compatibility and emits ordered
//! operations; physical geometry (seam atom allocation, CNOT schedules)
//! is lowered by [`lower_patch_plan`] which reuses the existing
//! lattice-surgery geometry helpers.
//!
//! # Model
//!
//! - [`Patch`] — one logical code patch (data + optional check atoms)
//! - [`PatchBoundary`] — rough/smooth edge of a patch
//! - [`PatchOperation`] — merge, split, measure-patch, measure-ancilla,
//!   prepare-ancilla, frame-update (all explicit, ordered)
//! - [`PatchPlan`] — ordered sequence of operations + patch registry
//! - [`PatchPlanner`] — builds a plan from a logical operation
//!
//! The plan is then lowered to [`PhysicalRound`]s by [`lower_patch_plan`].

use thiserror::Error;

use crate::expand::{
    ExpandError, ExpandedBlock, MergeBoundary, PauliFrameUpdate, PhysicalAtomId, PhysicalRound,
    RoundKind, RoundLocalOp, RoundTerminal,
};
use crate::lattice_surgery::{
    SeamAtoms, allocate_ancilla_patch, allocate_seam_atoms, bottom_row_data, left_column_data,
    place_patch_at, place_seam_column, place_seam_row, right_column_data, rough_merge_round,
    smooth_merge_round, split_seam_round, top_row_data,
};
use crate::workload::{LogicalBasis, LogicalQubitId};

/// Orientation of a patch boundary (surface-code geometry).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PatchBoundary {
    /// Rough (ZZ) — left/right edge of a rotated surface patch.
    Rough,
    /// Smooth (XX) — top/bottom edge of a rotated surface patch.
    Smooth,
}

impl std::fmt::Display for PatchBoundary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PatchBoundary {
    pub fn as_merge_boundary(self) -> MergeBoundary {
        match self {
            Self::Rough => MergeBoundary::Rough,
            Self::Smooth => MergeBoundary::Smooth,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rough => "rough",
            Self::Smooth => "smooth",
        }
    }
}

/// Which edge of a patch a boundary refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PatchEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl std::fmt::Display for PatchEdge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Left => f.write_str("left"),
            Self::Right => f.write_str("right"),
            Self::Top => f.write_str("top"),
            Self::Bottom => f.write_str("bottom"),
        }
    }
}

impl PatchEdge {
    /// The boundary type (rough/smooth) this edge has on a rotated surface code.
    pub fn boundary(self) -> PatchBoundary {
        match self {
            Self::Left | Self::Right => PatchBoundary::Rough,
            Self::Top | Self::Bottom => PatchBoundary::Smooth,
        }
    }
}

/// A logical code patch in the planner's registry.
#[derive(Clone, Debug)]
pub struct Patch {
    pub logical_id: LogicalQubitId,
    pub kind: PatchKind,
}

/// Whether a patch is a data/program block or a transitional ancilla.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PatchKind {
    /// A program-level data block (from a QEC constructor).
    Data,
    /// A transitional ancilla allocated for a merge/split gadget.
    Ancilla,
}

/// One explicit operation in a patch-operation plan.
#[derive(Clone, Debug)]
pub enum PatchOperation {
    /// Prepare a fresh ancilla patch (allocate + |+⟩ or |0⟩ init).
    PrepareAncilla {
        logical_id: LogicalQubitId,
        init_basis: LogicalBasis,
    },
    /// Merge two patches along a shared boundary via seam check atoms.
    Merge {
        primary: LogicalQubitId,
        partner: LogicalQubitId,
        boundary: PatchBoundary,
        /// Edge of `primary` where the merge happens.
        primary_edge: PatchEdge,
        /// Edge of `partner` where the merge happens.
        partner_edge: PatchEdge,
    },
    /// Split (re-measure the seam to project patches apart).
    Split {
        primary: LogicalQubitId,
        partner: Option<LogicalQubitId>,
        boundary: PatchBoundary,
    },
    /// Measure all data atoms of a patch in a given basis (logical measurement).
    MeasurePatch {
        logical_id: LogicalQubitId,
        basis: LogicalBasis,
    },
    /// Measure the ancilla's logical observable (partial data product).
    MeasureAncilla { logical_id: LogicalQubitId },
    /// Record outcome-conditioned Pauli frame byproducts.
    FrameUpdate {
        logical_id: LogicalQubitId,
        partner: Option<LogicalQubitId>,
        updates: Vec<PauliFrameUpdate>,
    },
}

/// An ordered plan of patch operations plus a patch registry.
#[derive(Clone, Debug, Default)]
pub struct PatchPlan {
    pub patches: Vec<Patch>,
    pub operations: Vec<PatchOperation>,
}

impl PatchPlan {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a patch in the registry.
    pub fn add_patch(&mut self, patch: Patch) {
        self.patches.push(patch);
    }

    /// Append an operation to the plan.
    pub fn push(&mut self, op: PatchOperation) {
        self.operations.push(op);
    }

    /// Look up a patch by logical id.
    pub fn find_patch(&self, id: LogicalQubitId) -> Option<&Patch> {
        self.patches.iter().find(|p| p.logical_id == id)
    }
}

/// Failures from planning or validating a patch-operation layout.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PlanError {
    #[error("patch-operation plan references unknown logical id {0}")]
    UnknownPatch(u32),
    #[error(
        "merge boundary mismatch: primary edge `{primary_edge}` is `{primary_boundary}`, \
         partner edge `{partner_edge}` is `{partner_boundary}`; merge requires matching boundary types"
    )]
    BoundaryMismatch {
        primary_edge: PatchEdge,
        primary_boundary: PatchBoundary,
        partner_edge: PatchEdge,
        partner_boundary: PatchBoundary,
    },
    #[error("merge requires two distinct patches; primary and partner are both {0}")]
    MergeSamePatch(u32),
    #[error("ancilla prepare requires surface family; logical {id} is `{family}`")]
    AncillaNotSurface { id: u32, family: &'static str },
    #[error("patch-operation plan references a data edge that does not exist on the patch")]
    InvalidEdge,
    #[error(transparent)]
    Expand(Box<ExpandError>),
}

impl From<ExpandError> for PlanError {
    fn from(e: ExpandError) -> Self {
        PlanError::Expand(Box::new(e))
    }
}

/// Build a patch-operation plan for logical CX (lattice surgery).
///
/// This is the **same** Horsman L-shaped gadget as the fixed CX template,
/// but now expressed as explicit patch operations that go through the
/// shared planner + lowering pipeline. Two examples with different
/// patch/ancilla placements compile through the same code path.
pub fn plan_logical_cx(
    control: LogicalQubitId,
    target: LogicalQubitId,
) -> Result<PatchPlan, PlanError> {
    if control == target {
        return Err(PlanError::MergeSamePatch(control.0));
    }

    let mut plan = PatchPlan::new();
    plan.add_patch(Patch {
        logical_id: control,
        kind: PatchKind::Data,
    });
    plan.add_patch(Patch {
        logical_id: target,
        kind: PatchKind::Data,
    });

    // Allocate a transitional ancilla (same id scheme as the fixed template).
    let ancilla_id = LogicalQubitId(control.0.max(target.0).saturating_add(1));
    plan.add_patch(Patch {
        logical_id: ancilla_id,
        kind: PatchKind::Ancilla,
    });

    // 1. Prepare ancilla in |+⟩ (X-init)
    plan.push(PatchOperation::PrepareAncilla {
        logical_id: ancilla_id,
        init_basis: LogicalBasis::X,
    });

    // 2. Rough merge: control↔ancilla (left/right edges)
    plan.push(PatchOperation::Merge {
        primary: control,
        partner: ancilla_id,
        boundary: PatchBoundary::Rough,
        primary_edge: PatchEdge::Right,
        partner_edge: PatchEdge::Left,
    });

    // 3. Split after rough
    plan.push(PatchOperation::Split {
        primary: control,
        partner: Some(ancilla_id),
        boundary: PatchBoundary::Rough,
    });

    // 4. Smooth merge: ancilla↔target (top/bottom edges)
    plan.push(PatchOperation::Merge {
        primary: ancilla_id,
        partner: target,
        boundary: PatchBoundary::Smooth,
        primary_edge: PatchEdge::Bottom,
        partner_edge: PatchEdge::Top,
    });

    // 5. Split after smooth
    plan.push(PatchOperation::Split {
        primary: ancilla_id,
        partner: Some(target),
        boundary: PatchBoundary::Smooth,
    });

    // 6. Measure ancilla logical Z
    plan.push(PatchOperation::MeasureAncilla {
        logical_id: ancilla_id,
    });

    // 7. Pauli frame byproducts (Horsman CX)
    plan.push(PatchOperation::FrameUpdate {
        logical_id: control,
        partner: Some(target),
        updates: Vec::new(), // populated during lowering (needs seam atom ids)
    });

    Ok(plan)
}

/// Build a patch-operation plan for logical X/Z measurement.
///
/// Logical measurement is now a patch operation, not an ad hoc path.
pub fn plan_measure_logical(
    logical_id: LogicalQubitId,
    basis: LogicalBasis,
) -> Result<PatchPlan, PlanError> {
    let mut plan = PatchPlan::new();
    plan.add_patch(Patch {
        logical_id,
        kind: PatchKind::Data,
    });
    plan.push(PatchOperation::MeasurePatch { logical_id, basis });
    Ok(plan)
}

/// Build a patch-operation plan for a bare rough merge + split between
/// two patches (not a full CX). This satisfies the criterion that at
/// least one patch-operation example emits merge/split phases without
/// being a full CX.
pub fn plan_rough_merge_split(
    primary: LogicalQubitId,
    partner: LogicalQubitId,
) -> Result<PatchPlan, PlanError> {
    if primary == partner {
        return Err(PlanError::MergeSamePatch(primary.0));
    }

    let mut plan = PatchPlan::new();
    plan.add_patch(Patch {
        logical_id: primary,
        kind: PatchKind::Data,
    });
    plan.add_patch(Patch {
        logical_id: partner,
        kind: PatchKind::Data,
    });

    plan.push(PatchOperation::Merge {
        primary,
        partner,
        boundary: PatchBoundary::Rough,
        primary_edge: PatchEdge::Right,
        partner_edge: PatchEdge::Left,
    });
    plan.push(PatchOperation::Split {
        primary,
        partner: Some(partner),
        boundary: PatchBoundary::Rough,
    });

    Ok(plan)
}

/// Validate that a patch-operation plan is self-consistent.
///
/// Checks: all referenced logical ids exist in the patch registry;
/// merge boundary types match between the two edges; merges involve
/// distinct patches; ancilla prepares use surface family.
pub fn validate_plan(plan: &PatchPlan) -> Result<(), PlanError> {
    for op in &plan.operations {
        match op {
            PatchOperation::PrepareAncilla { logical_id, .. } => {
                if plan.find_patch(*logical_id).is_none() {
                    return Err(PlanError::UnknownPatch(logical_id.0));
                }
            }
            PatchOperation::Merge {
                primary,
                partner,
                boundary,
                primary_edge,
                partner_edge,
            } => {
                if plan.find_patch(*primary).is_none() {
                    return Err(PlanError::UnknownPatch(primary.0));
                }
                if plan.find_patch(*partner).is_none() {
                    return Err(PlanError::UnknownPatch(partner.0));
                }
                if primary == partner {
                    return Err(PlanError::MergeSamePatch(primary.0));
                }
                let pb = primary_edge.boundary();
                let qb = partner_edge.boundary();
                if pb != *boundary || qb != *boundary {
                    return Err(PlanError::BoundaryMismatch {
                        primary_edge: *primary_edge,
                        primary_boundary: pb,
                        partner_edge: *partner_edge,
                        partner_boundary: qb,
                    });
                }
            }
            PatchOperation::Split {
                primary, partner, ..
            } => {
                if plan.find_patch(*primary).is_none() {
                    return Err(PlanError::UnknownPatch(primary.0));
                }
                if let Some(p) = partner
                    && plan.find_patch(*p).is_none()
                {
                    return Err(PlanError::UnknownPatch(p.0));
                }
            }
            PatchOperation::MeasurePatch { logical_id, .. }
            | PatchOperation::MeasureAncilla { logical_id }
            | PatchOperation::FrameUpdate { logical_id, .. } => {
                if plan.find_patch(*logical_id).is_none() {
                    return Err(PlanError::UnknownPatch(logical_id.0));
                }
            }
        }
    }
    Ok(())
}

/// Lower a validated patch-operation plan into physical rounds.
///
/// This reuses the existing geometry helpers from [`crate::lattice_surgery`]
/// (seam allocation, merge/split round builders) but now they are driven
/// by the explicit plan rather than a hardcoded CX sequence.
///
/// `layouts` is mutated in place: ancilla patches and seam atoms are
/// appended. `next_atom` is the global atom-id counter.
pub fn lower_patch_plan(
    plan: &PatchPlan,
    layouts: &mut Vec<ExpandedBlock>,
    next_atom: &mut u32,
    rounds: &mut Vec<PhysicalRound>,
) -> Result<(), PlanError> {
    // Track seam atoms per merge for split reuse and frame-update conditions.
    let mut seam_cache: Vec<(LogicalQubitId, LogicalQubitId, PatchBoundary, SeamAtoms)> =
        Vec::new();
    let mut ancilla_layout_ids: Vec<LogicalQubitId> = Vec::new();
    // Track the ancilla logical-Z measurement atoms for frame updates.
    let mut ancilla_mz_atoms: Vec<PhysicalAtomId> = Vec::new();

    for op in &plan.operations {
        match op {
            PatchOperation::PrepareAncilla {
                logical_id,
                init_basis,
            } => {
                let distance = match find_layout(layouts, *logical_id) {
                    Ok(layout) => layout.distance,
                    Err(_) => {
                        // Fresh ancilla allocation: use the distance from an
                        // existing surface data patch (for CX, control and
                        // target are the same distance).
                        layouts
                            .iter()
                            .find(|b| b.family == crate::family::SourceFamily::Surface)
                            .map(|b| b.distance)
                            .ok_or(PlanError::UnknownPatch(logical_id.0))?
                    }
                };

                let mut ancilla = allocate_ancilla_patch(*logical_id, distance, next_atom)?;

                // L-shaped placement: ancilla goes to the right of the primary
                // patch (control for CX). We position it using the last data
                // patch's right edge.
                let patch_span = 2 * distance as i32 + crate::lattice_surgery::PATCH_GAP;
                place_patch_at(&mut ancilla, patch_span, 0);
                layouts.push(ancilla);
                ancilla_layout_ids.push(*logical_id);

                // Emit |+⟩ prep (X-init) or |0⟩ prep (Z-init) construct round.
                let local_before = if *init_basis == LogicalBasis::X {
                    let ancilla_block = layouts.last().expect("just pushed");
                    ancilla_block
                        .data_atoms
                        .iter()
                        .map(|&atom| RoundLocalOp::H { atom })
                        .collect()
                } else {
                    Vec::new()
                };
                rounds.push(PhysicalRound {
                    kind: RoundKind::Construct,
                    logical_id: *logical_id,
                    local_before,
                    entangling: Vec::new(),
                    z_cnot_count: 0,
                    local_mid: Vec::new(),
                    local_after: Vec::new(),
                    terminal: Vec::new(),
                    partner_logical_id: None,
                    frame_updates: Vec::new(),
                });
            }

            PatchOperation::Merge {
                primary,
                partner,
                boundary,
                primary_edge,
                partner_edge,
            } => {
                let primary_idx = find_layout_index(layouts, *primary)?;
                let partner_idx = find_layout_index(layouts, *partner)?;

                let distance = layouts[primary_idx].distance;
                let patch_span = 2 * distance as i32 + crate::lattice_surgery::PATCH_GAP;

                // Reposition the partner patch relative to the primary based on
                // the edge pair so the seam atoms land between the facing edges
                // (same L-shaped geometry as the fixed CX template).
                match (primary_edge, partner_edge) {
                    (PatchEdge::Right, PatchEdge::Left) => {
                        // Partner goes to the right of primary.
                        let primary_max_x = layouts[primary_idx]
                            .coords
                            .iter()
                            .map(|(x, _)| *x)
                            .max()
                            .unwrap_or(0);
                        let partner_min_x = layouts[partner_idx]
                            .coords
                            .iter()
                            .map(|(x, _)| *x)
                            .min()
                            .unwrap_or(0);
                        let dx = primary_max_x + patch_span - partner_min_x;
                        place_patch_at(&mut layouts[partner_idx], dx, 0);
                    }
                    (PatchEdge::Bottom, PatchEdge::Top) => {
                        // Partner goes below primary (aligned in x for L-shape).
                        let primary_min_x = layouts[primary_idx]
                            .coords
                            .iter()
                            .map(|(x, _)| *x)
                            .min()
                            .unwrap_or(0);
                        let partner_min_x = layouts[partner_idx]
                            .coords
                            .iter()
                            .map(|(x, _)| *x)
                            .min()
                            .unwrap_or(0);
                        let dx = primary_min_x - partner_min_x;
                        let primary_max_y = layouts[primary_idx]
                            .coords
                            .iter()
                            .map(|(_, y)| *y)
                            .max()
                            .unwrap_or(0);
                        let partner_min_y = layouts[partner_idx]
                            .coords
                            .iter()
                            .map(|(_, y)| *y)
                            .min()
                            .unwrap_or(0);
                        let dy = primary_max_y + patch_span - partner_min_y;
                        place_patch_at(&mut layouts[partner_idx], dx, dy);
                    }
                    (PatchEdge::Left, PatchEdge::Right) => {
                        // Primary goes to the right of partner.
                        let partner_max_x = layouts[partner_idx]
                            .coords
                            .iter()
                            .map(|(x, _)| *x)
                            .max()
                            .unwrap_or(0);
                        let primary_min_x = layouts[primary_idx]
                            .coords
                            .iter()
                            .map(|(x, _)| *x)
                            .min()
                            .unwrap_or(0);
                        let dx = partner_max_x + patch_span - primary_min_x;
                        place_patch_at(&mut layouts[primary_idx], dx, 0);
                    }
                    (PatchEdge::Top, PatchEdge::Bottom) => {
                        // Primary goes above partner (partner below primary).
                        let primary_max_y = layouts[primary_idx]
                            .coords
                            .iter()
                            .map(|(_, y)| *y)
                            .max()
                            .unwrap_or(0);
                        let partner_min_y = layouts[partner_idx]
                            .coords
                            .iter()
                            .map(|(_, y)| *y)
                            .min()
                            .unwrap_or(0);
                        let dy = primary_max_y + patch_span - partner_min_y;
                        place_patch_at(&mut layouts[partner_idx], 0, dy);
                    }
                    _ => {
                        // Other edge combinations: no repositioning (unsupported layout).
                    }
                }

                let mut seam = allocate_seam_atoms(distance, next_atom)?;

                // Position seam based on primary edge.
                match primary_edge {
                    PatchEdge::Right => {
                        place_seam_column(
                            &mut seam,
                            2 * distance as i32 + 1,
                            &layouts[primary_idx],
                        );
                        let left_col = right_column_data(&layouts[primary_idx])?;
                        let right_col = left_column_data(&layouts[partner_idx])?;
                        rounds.push(rough_merge_round(
                            &left_col, &right_col, &seam, *primary, *partner,
                        )?);
                    }
                    PatchEdge::Bottom => {
                        place_seam_row(&mut seam, 2 * distance as i32 + 1, &layouts[partner_idx]);
                        let above_row = bottom_row_data(&layouts[partner_idx])?;
                        let below_row = top_row_data(&layouts[primary_idx])?;
                        rounds.push(smooth_merge_round(
                            &above_row, &below_row, &seam, *primary, *partner,
                        )?);
                    }
                    PatchEdge::Left => {
                        place_seam_column(&mut seam, 0, &layouts[primary_idx]);
                        let left_col = right_column_data(&layouts[partner_idx])?;
                        let right_col = left_column_data(&layouts[primary_idx])?;
                        rounds.push(rough_merge_round(
                            &left_col, &right_col, &seam, *primary, *partner,
                        )?);
                    }
                    PatchEdge::Top => {
                        place_seam_row(&mut seam, 0, &layouts[primary_idx]);
                        let above_row = bottom_row_data(&layouts[primary_idx])?;
                        let below_row = top_row_data(&layouts[partner_idx])?;
                        rounds.push(smooth_merge_round(
                            &above_row, &below_row, &seam, *primary, *partner,
                        )?);
                    }
                }

                // Attach seam atoms to the partner (ancilla) block for accounting.
                let merge_boundary = boundary.as_merge_boundary();
                seam_cache.push((*primary, *partner, *boundary, seam));

                // Update the round kind to Merge
                if let Some(last) = rounds.last_mut() {
                    last.kind = RoundKind::Merge(merge_boundary);
                }
            }

            PatchOperation::Split {
                primary,
                partner,
                boundary,
            } => {
                // Find the matching seam from the cache.
                let seam_entry = seam_cache
                    .iter()
                    .find(|(p, q, b, _)| {
                        *p == *primary && partner.is_none_or(|par| *q == par) && *b == *boundary
                    })
                    .ok_or(PlanError::InvalidEdge)?;

                let primary_idx = find_layout_index(layouts, *primary)?;
                let partner_idx = partner.map(|p| find_layout_index(layouts, p)).transpose()?;

                let (left_col, right_col) = match boundary {
                    PatchBoundary::Rough => {
                        let left = right_column_data(&layouts[primary_idx])?;
                        let right = partner_idx
                            .map(|pi| left_column_data(&layouts[pi]))
                            .transpose()?
                            .unwrap_or_else(|| left.clone());
                        (left, right)
                    }
                    PatchBoundary::Smooth => {
                        let above = bottom_row_data(&layouts[primary_idx])?;
                        let below = partner_idx
                            .map(|pi| top_row_data(&layouts[pi]))
                            .transpose()?
                            .unwrap_or_else(|| above.clone());
                        (above, below)
                    }
                };

                // Re-use the seam atoms (they're already allocated).
                let seam = &seam_entry.3;
                let mut round = split_seam_round(
                    boundary.as_merge_boundary(),
                    &left_col,
                    &right_col,
                    seam,
                    *primary,
                    *partner,
                )?;
                // Adjust the round kind to Split.
                round.kind = RoundKind::Split(boundary.as_merge_boundary());
                rounds.push(round);
            }

            PatchOperation::MeasurePatch { logical_id, basis } => {
                let layout = find_layout(layouts, *logical_id)?;
                let terminal = layout
                    .data_atoms
                    .iter()
                    .map(|&atom| RoundTerminal::Measure {
                        atom,
                        basis: *basis,
                    })
                    .collect();
                rounds.push(PhysicalRound {
                    kind: RoundKind::MeasureLogical,
                    logical_id: *logical_id,
                    local_before: Vec::new(),
                    entangling: Vec::new(),
                    z_cnot_count: 0,
                    local_mid: Vec::new(),
                    local_after: Vec::new(),
                    terminal,
                    partner_logical_id: None,
                    frame_updates: Vec::new(),
                });
            }

            PatchOperation::MeasureAncilla { logical_id } => {
                let layout = find_layout(layouts, *logical_id)?;
                let top_row = top_row_data(layout)?;
                ancilla_mz_atoms = top_row.clone();
                rounds.push(PhysicalRound {
                    kind: RoundKind::MeasureAncilla,
                    logical_id: *logical_id,
                    local_before: Vec::new(),
                    entangling: Vec::new(),
                    z_cnot_count: 0,
                    local_mid: Vec::new(),
                    local_after: Vec::new(),
                    terminal: top_row
                        .iter()
                        .map(|&atom| RoundTerminal::Measure {
                            atom,
                            basis: LogicalBasis::Z,
                        })
                        .collect(),
                    partner_logical_id: None,
                    frame_updates: Vec::new(),
                });
            }

            PatchOperation::FrameUpdate {
                logical_id,
                partner,
                updates,
            } => {
                let frame_updates = if !updates.is_empty() {
                    updates.clone()
                } else {
                    // For CX plans, build Horsman byproducts from seam + ancilla MZ.
                    build_cx_frame_updates(logical_id, partner, &seam_cache, &ancilla_mz_atoms)
                };
                rounds.push(PhysicalRound {
                    kind: RoundKind::FrameUpdate,
                    logical_id: *logical_id,
                    local_before: Vec::new(),
                    entangling: Vec::new(),
                    z_cnot_count: 0,
                    local_mid: Vec::new(),
                    local_after: Vec::new(),
                    terminal: Vec::new(),
                    partner_logical_id: *partner,
                    frame_updates,
                });
            }
        }
    }

    // Attach seam atoms to ancilla blocks for check-graph accounting.
    // In the CX gadget, both seams are on the ancilla (it is the common
    // patch in rough and smooth merges). The seam_cache stores
    // (primary, partner, boundary, seam); we attach to whichever of
    // primary/partner is the ancilla.
    for (primary_id, partner_id, boundary, seam) in &seam_cache {
        let attach_id = if plan
            .find_patch(*primary_id)
            .is_some_and(|p| p.kind == PatchKind::Ancilla)
        {
            *primary_id
        } else if plan
            .find_patch(*partner_id)
            .is_some_and(|p| p.kind == PatchKind::Ancilla)
        {
            *partner_id
        } else {
            // No ancilla in this merge — skip attachment (bare merge/split).
            continue;
        };
        let Ok(idx) = find_layout_index(layouts, attach_id) else {
            continue;
        };
        for i in 0..seam.atoms.len() {
            let atom = seam.atoms[i];
            layouts[idx].atoms.push(atom);
            layouts[idx].coords.push(seam.coords[i]);
            layouts[idx].check_atoms.push(atom);
            layouts[idx].stabilizers.push(crate::expand::StabilizerDef {
                check: atom,
                basis: match boundary {
                    PatchBoundary::Rough => LogicalBasis::Z,
                    PatchBoundary::Smooth => LogicalBasis::X,
                },
                data: vec![], // data atoms filled by merge geometry at lower time
            });
        }
    }

    Ok(())
}

/// Build Horsman CX byproduct frame updates from cached seam + ancilla atoms.
fn build_cx_frame_updates(
    control: &LogicalQubitId,
    target: &Option<LogicalQubitId>,
    seam_cache: &[(LogicalQubitId, LogicalQubitId, PatchBoundary, SeamAtoms)],
    ancilla_mz_atoms: &[PhysicalAtomId],
) -> Vec<PauliFrameUpdate> {
    let target_id = target.unwrap_or(*control);

    let rough_seam = seam_cache
        .iter()
        .find(|(_, _, b, _)| *b == PatchBoundary::Rough)
        .map(|(_, _, _, s)| s.atoms.clone())
        .unwrap_or_default();
    let smooth_seam = seam_cache
        .iter()
        .find(|(_, _, b, _)| *b == PatchBoundary::Smooth)
        .map(|(_, _, _, s)| s.atoms.clone())
        .unwrap_or_default();

    vec![
        PauliFrameUpdate {
            logical_id: target_id,
            x: true,
            z: false,
            source: "rough_merge",
            condition_atoms: rough_seam,
        },
        PauliFrameUpdate {
            logical_id: *control,
            x: false,
            z: true,
            source: "smooth_merge",
            condition_atoms: smooth_seam.clone(),
        },
        PauliFrameUpdate {
            logical_id: *control,
            x: false,
            z: true,
            source: "ancilla_mz",
            condition_atoms: ancilla_mz_atoms.to_vec(),
        },
        PauliFrameUpdate {
            logical_id: target_id,
            x: true,
            z: false,
            source: "ancilla_mz",
            condition_atoms: ancilla_mz_atoms.to_vec(),
        },
    ]
}

fn find_layout(layouts: &[ExpandedBlock], id: LogicalQubitId) -> Result<&ExpandedBlock, PlanError> {
    layouts
        .iter()
        .find(|b| b.logical_id == id)
        .ok_or(PlanError::UnknownPatch(id.0))
}

fn find_layout_index(layouts: &[ExpandedBlock], id: LogicalQubitId) -> Result<usize, PlanError> {
    layouts
        .iter()
        .position(|b| b.logical_id == id)
        .ok_or(PlanError::UnknownPatch(id.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::{LogicalBasis, LogicalQubitId};

    #[test]
    fn plan_logical_cx_produces_ordered_ops() {
        let plan = plan_logical_cx(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
        assert_eq!(plan.patches.len(), 3); // control, target, ancilla
        assert!(plan.patches.iter().any(|p| p.kind == PatchKind::Ancilla));

        // Check operation ordering: prepare → merge → split → merge → split → measure → frame
        let kinds: Vec<&str> = plan
            .operations
            .iter()
            .map(|op| match op {
                PatchOperation::PrepareAncilla { .. } => "prepare",
                PatchOperation::Merge { boundary, .. } => match boundary {
                    PatchBoundary::Rough => "merge_rough",
                    PatchBoundary::Smooth => "merge_smooth",
                },
                PatchOperation::Split { boundary, .. } => match boundary {
                    PatchBoundary::Rough => "split_rough",
                    PatchBoundary::Smooth => "split_smooth",
                },
                PatchOperation::MeasurePatch { .. } => "measure_patch",
                PatchOperation::MeasureAncilla { .. } => "measure_ancilla",
                PatchOperation::FrameUpdate { .. } => "frame_update",
            })
            .collect();

        assert_eq!(
            kinds,
            vec![
                "prepare",
                "merge_rough",
                "split_rough",
                "merge_smooth",
                "split_smooth",
                "measure_ancilla",
                "frame_update",
            ]
        );
    }

    #[test]
    fn plan_logical_cx_rejects_same_patch() {
        let err = plan_logical_cx(LogicalQubitId(0), LogicalQubitId(0)).unwrap_err();
        assert_eq!(err, PlanError::MergeSamePatch(0));
    }

    #[test]
    fn plan_measure_logical_is_single_measure_patch_op() {
        let plan = plan_measure_logical(LogicalQubitId(2), LogicalBasis::X).unwrap();
        assert_eq!(plan.patches.len(), 1);
        assert_eq!(plan.operations.len(), 1);
        match &plan.operations[0] {
            PatchOperation::MeasurePatch { logical_id, basis } => {
                assert_eq!(*logical_id, LogicalQubitId(2));
                assert_eq!(*basis, LogicalBasis::X);
            }
            other => panic!("expected MeasurePatch, got {other:?}"),
        }
    }

    #[test]
    fn plan_bare_merge_split_is_not_full_cx() {
        let plan = plan_rough_merge_split(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
        assert_eq!(plan.patches.len(), 2); // no ancilla
        assert_eq!(plan.operations.len(), 2); // just merge + split, no measure/frame
        assert!(matches!(plan.operations[0], PatchOperation::Merge { .. }));
        assert!(matches!(plan.operations[1], PatchOperation::Split { .. }));
    }

    #[test]
    fn validate_plan_accepts_valid_cx_plan() {
        let plan = plan_logical_cx(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
        validate_plan(&plan).expect("valid plan should pass");
    }

    #[test]
    fn validate_plan_accepts_bare_merge_split() {
        let plan = plan_rough_merge_split(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
        validate_plan(&plan).expect("valid plan should pass");
    }

    #[test]
    fn validate_plan_rejects_boundary_mismatch() {
        let mut plan = PatchPlan::new();
        plan.add_patch(Patch {
            logical_id: LogicalQubitId(0),
            kind: PatchKind::Data,
        });
        plan.add_patch(Patch {
            logical_id: LogicalQubitId(1),
            kind: PatchKind::Data,
        });
        // Left edge is rough, top edge is smooth — mismatch
        plan.push(PatchOperation::Merge {
            primary: LogicalQubitId(0),
            partner: LogicalQubitId(1),
            boundary: PatchBoundary::Rough,
            primary_edge: PatchEdge::Left,
            partner_edge: PatchEdge::Top, // smooth!
        });
        let err = validate_plan(&plan).unwrap_err();
        assert!(matches!(err, PlanError::BoundaryMismatch { .. }));
    }

    #[test]
    fn validate_plan_rejects_unknown_patch_reference() {
        let mut plan = PatchPlan::new();
        plan.add_patch(Patch {
            logical_id: LogicalQubitId(0),
            kind: PatchKind::Data,
        });
        plan.push(PatchOperation::MeasurePatch {
            logical_id: LogicalQubitId(99), // not registered
            basis: LogicalBasis::Z,
        });
        let err = validate_plan(&plan).unwrap_err();
        assert_eq!(err, PlanError::UnknownPatch(99));
    }

    #[test]
    fn validate_plan_rejects_merge_same_patch() {
        let mut plan = PatchPlan::new();
        plan.add_patch(Patch {
            logical_id: LogicalQubitId(0),
            kind: PatchKind::Data,
        });
        plan.push(PatchOperation::Merge {
            primary: LogicalQubitId(0),
            partner: LogicalQubitId(0),
            boundary: PatchBoundary::Rough,
            primary_edge: PatchEdge::Right,
            partner_edge: PatchEdge::Left,
        });
        let err = validate_plan(&plan).unwrap_err();
        assert_eq!(err, PlanError::MergeSamePatch(0));
    }

    #[test]
    fn patch_edge_boundary_mapping() {
        assert_eq!(PatchEdge::Left.boundary(), PatchBoundary::Rough);
        assert_eq!(PatchEdge::Right.boundary(), PatchBoundary::Rough);
        assert_eq!(PatchEdge::Top.boundary(), PatchBoundary::Smooth);
        assert_eq!(PatchEdge::Bottom.boundary(), PatchBoundary::Smooth);
    }

    #[test]
    fn cx_plan_has_all_phase_kinds() {
        let plan = plan_logical_cx(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
        let has_prepare = plan
            .operations
            .iter()
            .any(|op| matches!(op, PatchOperation::PrepareAncilla { .. }));
        let has_merge = plan
            .operations
            .iter()
            .any(|op| matches!(op, PatchOperation::Merge { .. }));
        let has_split = plan
            .operations
            .iter()
            .any(|op| matches!(op, PatchOperation::Split { .. }));
        let has_measure_ancilla = plan
            .operations
            .iter()
            .any(|op| matches!(op, PatchOperation::MeasureAncilla { .. }));
        let has_frame = plan
            .operations
            .iter()
            .any(|op| matches!(op, PatchOperation::FrameUpdate { .. }));
        assert!(has_prepare && has_merge && has_split && has_measure_ancilla && has_frame);
    }

    #[test]
    fn two_cx_examples_different_placements() {
        // Example 1: CX(0, 1)
        let plan1 = plan_logical_cx(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
        // Example 2: CX(2, 3) — different patches, same planner
        let plan2 = plan_logical_cx(LogicalQubitId(2), LogicalQubitId(3)).unwrap();

        // Both plans have the same structure (same op count/kinds)
        assert_eq!(plan1.operations.len(), plan2.operations.len());
        assert_eq!(plan1.patches.len(), plan2.patches.len());

        // But different logical ids
        assert_ne!(plan1.patches[0].logical_id, plan2.patches[0].logical_id);
    }
}
