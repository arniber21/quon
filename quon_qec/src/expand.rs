//! Hybrid QEC expansion: workload → per-round physical gate graphs (ADR-0016).
//!
//! Produces MLIR-free physical rounds that `quon_na` schedules with existing
//! planners *inside* each round. Round boundaries are first-class so NA
//! compaction can attach Barrier edges and must not reorder across memory
//! rounds.
//!
//! # Repetition ([`SourceFamily::Repetition`])
//!
//! Alternating `D C D C … D` chain of length `N = 2d − 1` (architecture_model
//! §10.2 / [Kelly15]). Each check extracts ZZ parity of its two neighboring
//! data qubits via CNOT(data→check), then Z-measure + reset.
//!
//! # Surface ([`SourceFamily::Surface`])
//!
//! Rotated surface code with `N = 2d² − 1` (architecture_model §10.1 /
//! [Bravyi24] §1; [BMD07]). Data on a `d×d` grid; X/Z check ancillas on
//! plaquettes (smooth top/bottom X boundaries, rough left/right Z). Memory
//! rounds use a **serial Z-then-X** phase split (Z CXs → mid H → X CXs →
//! after H → measure/reset) for hybrid NA scheduling. That is *not* Stim's
//! interleaved 4-layer schedule — do not claim Stim-equivalent fault-tolerant
//! distance from this expansion alone.
//!
//! `logical_cx` expands via fixed-layout three-patch lattice surgery
//! ([`crate::lattice_surgery`], ADR-0019 / #250).
//!
//! `surface_code_x` / X-init constructs emit data Hadamards (logical |+⟩ prep)
//! on the construct round before memory rounds.

use thiserror::Error;

use crate::family::{CodeFamily, QecError, SourceFamily, repetition_n, surface_n};
use crate::workload::{LogicalBasis, LogicalQubitId, QecWorkload, WorkloadBlock, WorkloadOp};

/// Dense physical atom id within an expanded layout (0-based globally).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalAtomId(pub u32);

/// CNOT for syndrome extraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhysicalCnot {
    pub control: PhysicalAtomId,
    pub target: PhysicalAtomId,
}

/// Single-qubit op scheduled around entangling (Stim + expand IR; NA planners
/// skip these — microwave/Raman locals need no AOD place/move).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundLocalOp {
    H { atom: PhysicalAtomId },
}

/// Post-entangling measure or reset on one atom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundTerminal {
    Measure {
        atom: PhysicalAtomId,
        basis: LogicalBasis,
    },
    Reset {
        atom: PhysicalAtomId,
    },
}

/// Smooth (XX) vs rough (ZZ) merge boundary in lattice surgery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MergeBoundary {
    /// Joint ZZ measurement along the shared rough edge.
    Rough,
    /// Joint XX measurement along the shared smooth edge.
    Smooth,
}

impl MergeBoundary {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rough => "rough",
            Self::Smooth => "smooth",
        }
    }
}

/// Pauli frame byproduct recorded after lattice-surgery measurements.
///
/// `x` / `z` name the Pauli to apply on [`Self::logical_id`] when the parity of
/// [`Self::condition_atoms`] measurement records is odd (−1). They are **not**
/// unconditional correction flags.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PauliFrameUpdate {
    pub logical_id: LogicalQubitId,
    pub x: bool,
    pub z: bool,
    /// Which merge/measure phase produced this byproduct (`rough_merge`, …).
    pub source: &'static str,
    /// Measurement atoms whose record parity conditions this update (odd → apply).
    pub condition_atoms: Vec<PhysicalAtomId>,
}

/// Why this physical round exists in the expanded schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoundKind {
    /// Block allocation / preparation (no entangling in v0).
    Construct,
    /// One syndrome-extraction memory round.
    MemoryRound,
    /// Final logical Pauli measurement (consumes the block).
    MeasureLogical,
    /// Lattice-surgery merge (joint stabilizers across a patch seam).
    Merge(MergeBoundary),
    /// Lattice-surgery split (barrier; patches resume separately).
    Split(MergeBoundary),
    /// Measure transitional ancilla after CX gadget (byproduct source).
    MeasureAncilla,
    /// Record Pauli frame byproducts (no physical gates).
    FrameUpdate,
    /// Magic-state-consuming logical T (issue #283).
    /// Compiler model only — no physical gate expansion.
    MagicT,
    /// Magic-state-consuming logical T† (issue #283).
    MagicTdag,
    /// Magic-state-consuming logical CCZ (issue #283).
    MagicCcz,
}

impl RoundKind {
    /// Rounds that get a durable Wait barrier in the hybrid NA schedule.
    pub fn needs_round_barrier(self) -> bool {
        matches!(
            self,
            Self::MemoryRound
                | Self::Merge(_)
                | Self::Split(_)
                | Self::MeasureAncilla
                | Self::FrameUpdate
        )
    }

    pub fn as_experiment_str(self) -> &'static str {
        match self {
            Self::Construct => "construct",
            Self::MemoryRound => "memory_round",
            Self::MeasureLogical => "measure_logical",
            Self::Merge(MergeBoundary::Rough) => "merge_rough",
            Self::Merge(MergeBoundary::Smooth) => "merge_smooth",
            Self::Split(MergeBoundary::Rough) => "split_rough",
            Self::Split(MergeBoundary::Smooth) => "split_smooth",
            Self::MeasureAncilla => "measure_ancilla",
            Self::FrameUpdate => "frame_update",
            Self::MagicT => "magic_t",
            Self::MagicTdag => "magic_tdag",
            Self::MagicCcz => "magic_ccz",
        }
    }
}

/// One schedulable unit: optional locals, entangling CNOTs, terminals.
///
/// `quon_na` schedules Z-then-X phases using [`Self::z_cnot_count`] /
/// [`Self::local_mid`] / [`Self::local_after`], emitting LocalGate Hadamards
/// (not silently dropped). Surface rounds: first [`Self::z_cnot_count`] CNOTs
/// are Z-plaquette extraction; mid Hadamards then remaining X-plaquette CNOTs;
/// after closes the X sandwich. Repetition leaves
/// `z_cnot_count == entangling.len()` and mid empty.
///
/// **FT-distance note:** this serial Z-then-X split is for hybrid NA scheduling
/// fidelity to expand IR — it is not Stim's interleaved 4-layer extraction and
/// must not be claimed Stim-equivalent for threshold / distance arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicalRound {
    pub kind: RoundKind,
    pub logical_id: LogicalQubitId,
    pub local_before: Vec<RoundLocalOp>,
    pub entangling: Vec<PhysicalCnot>,
    /// Prefix of [`Self::entangling`] belonging to Z-phase extraction.
    pub z_cnot_count: usize,
    pub local_mid: Vec<RoundLocalOp>,
    pub local_after: Vec<RoundLocalOp>,
    pub terminal: Vec<RoundTerminal>,
    /// Second patch involved in a merge/split (lattice surgery).
    pub partner_logical_id: Option<LogicalQubitId>,
    /// Pauli frame byproducts (populated on [`RoundKind::FrameUpdate`]).
    pub frame_updates: Vec<PauliFrameUpdate>,
}

impl PhysicalRound {
    /// Empty physical content for construct / split / frame-update rounds.
    pub fn bare(kind: RoundKind, logical_id: LogicalQubitId) -> Self {
        Self {
            kind,
            logical_id,
            local_before: Vec::new(),
            entangling: Vec::new(),
            z_cnot_count: 0,
            local_mid: Vec::new(),
            local_after: Vec::new(),
            terminal: Vec::new(),
            partner_logical_id: None,
            frame_updates: Vec::new(),
        }
    }
}

/// One stabilizer generator in an expanded block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StabilizerDef {
    pub check: PhysicalAtomId,
    /// Pauli type of the stabilizer (`X` or `Z` plaquette).
    pub basis: LogicalBasis,
    pub data: Vec<PhysicalAtomId>,
}

/// Per-block atom layout after expansion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedBlock {
    pub logical_id: LogicalQubitId,
    pub family: SourceFamily,
    pub code_family: CodeFamily,
    pub distance: u32,
    pub init_basis: LogicalBasis,
    /// Full atom list in layout order.
    pub atoms: Vec<PhysicalAtomId>,
    pub data_atoms: Vec<PhysicalAtomId>,
    pub check_atoms: Vec<PhysicalAtomId>,
    /// Lattice coordinates parallel to [`Self::atoms`] (Stim `QUBIT_COORDS`).
    pub coords: Vec<(i32, i32)>,
    pub stabilizers: Vec<StabilizerDef>,
}

/// Fully expanded QEC workload ready for hybrid NA scheduling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedWorkload {
    pub blocks: Vec<ExpandedBlock>,
    pub rounds: Vec<PhysicalRound>,
}

impl ExpandedWorkload {
    /// Total physical atoms across all blocks.
    pub fn physical_atom_count(&self) -> usize {
        self.blocks.iter().map(|b| b.atoms.len()).sum()
    }

    /// Number of [`RoundKind::MemoryRound`] rounds.
    pub fn memory_round_count(&self) -> usize {
        self.rounds
            .iter()
            .filter(|r| r.kind == RoundKind::MemoryRound)
            .count()
    }

    /// Rounds that emit a durable Wait barrier in the hybrid NA schedule.
    pub fn barrier_round_count(&self) -> usize {
        self.rounds
            .iter()
            .filter(|r| r.kind.needs_round_barrier())
            .count()
    }

    /// Whether this expansion includes lattice-surgery CX phases.
    pub fn has_lattice_surgery(&self) -> bool {
        self.rounds.iter().any(|r| {
            matches!(
                r.kind,
                RoundKind::Merge(_) | RoundKind::Split(_) | RoundKind::MeasureAncilla
            )
        })
    }

    /// Count of logical T gates (magic-state-consuming, issue #283).
    pub fn t_count(&self) -> usize {
        self.rounds
            .iter()
            .filter(|r| r.kind == RoundKind::MagicT)
            .count()
    }

    /// Count of logical T† gates (issue #283).
    pub fn tdag_count(&self) -> usize {
        self.rounds
            .iter()
            .filter(|r| r.kind == RoundKind::MagicTdag)
            .count()
    }

    /// Count of logical CCZ gates (issue #283).
    pub fn ccz_count(&self) -> usize {
        self.rounds
            .iter()
            .filter(|r| r.kind == RoundKind::MagicCcz)
            .count()
    }

    /// Total magic-state demand (T + Tdag + CCZ, issue #283).
    pub fn magic_state_demand(&self) -> usize {
        self.t_count() + self.tdag_count() + self.ccz_count()
    }
}

/// Failures expanding a [`QecWorkload`] into physical rounds.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ExpandError {
    #[error(transparent)]
    Qec(#[from] QecError),
    #[error("unknown logical qubit id {0} in workload op")]
    UnknownLogicalId(u32),
    #[error("logical_cx requires surface-code blocks; logical {id} is `{family}`")]
    LogicalCxNotSurface { id: u32, family: &'static str },
    #[error(
        "logical_cx requires equal distances; control distance {control_distance}, target distance {target_distance}"
    )]
    LogicalCxDistanceMismatch {
        control_distance: u32,
        target_distance: u32,
    },
    #[error("expanded atom id overflow")]
    AtomIdOverflow,
    #[error(
        "repetition-code construct requires Z init basis (ADR-0014); got `{basis}` \
         for logical {logical_id}"
    )]
    NonZInitBasis {
        logical_id: u32,
        basis: &'static str,
    },
    #[error(
        "surface-code stabilizer layout invalid for distance {distance}: \
         expected {expected} checks, got {got}"
    )]
    InvalidSurfaceLayout {
        distance: u32,
        expected: u32,
        got: usize,
    },
    #[error(
        "z_cnot_count ({z_cnot_count}) exceeds entangling.len() ({entangling_len}); \
         refusing silent clamp"
    )]
    InvalidZCnotCount {
        z_cnot_count: usize,
        entangling_len: usize,
    },
    #[error("surface patch data count mismatch for distance {distance}")]
    InvalidPatchData { distance: u32 },
}

/// Expand a validated [`QecWorkload`] into per-round physical CNOT / measure / reset.
pub fn expand_workload(workload: &QecWorkload) -> Result<ExpandedWorkload, ExpandError> {
    let mut next_atom = 0u32;
    let mut layouts: Vec<ExpandedBlock> = Vec::with_capacity(workload.blocks.len());

    for meta in &workload.blocks {
        layouts.push(expand_block_layout(meta, &mut next_atom)?);
    }

    let mut rounds = Vec::new();
    for op in &workload.ops {
        match op {
            WorkloadOp::Construct {
                logical_id,
                family,
                basis,
                ..
            } => {
                let layout = find_layout(&layouts, *logical_id)?;
                if *family == SourceFamily::Repetition
                    && (*basis != LogicalBasis::Z || layout.init_basis != LogicalBasis::Z)
                {
                    return Err(ExpandError::NonZInitBasis {
                        logical_id: logical_id.0,
                        basis: basis.as_str(),
                    });
                }
                // Construct allocates. X-init surface emits data Hadamards
                // (logical |+⟩ prep); Z-init is |0…0⟩ with no locals.
                let local_before = if *basis == LogicalBasis::X {
                    layout
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
            WorkloadOp::MemoryRound { logical_id } => {
                let layout = find_layout(&layouts, *logical_id)?;
                let round = match layout.family {
                    SourceFamily::Repetition => repetition_memory_round(layout)?,
                    SourceFamily::Surface => surface_memory_round(layout),
                };
                validate_round_z_cnot(&round)?;
                rounds.push(round);
            }
            WorkloadOp::MeasureLogical { logical_id, basis } => {
                let layout = find_layout(&layouts, *logical_id)?;
                rounds.push(measure_logical_round(layout, *basis));
            }
            WorkloadOp::LogicalCx { control, target } => {
                crate::lattice_surgery::expand_logical_cx(
                    *control,
                    *target,
                    &mut layouts,
                    &mut next_atom,
                    &mut rounds,
                )?;
            }
            WorkloadOp::LogicalT { logical_id } => {
                // Magic-state-consuming T: compiler model only (issue #283).
                // No physical gate expansion — records magic-state consumption.
                let _ = find_layout(&layouts, *logical_id)?;
                rounds.push(PhysicalRound::bare(RoundKind::MagicT, *logical_id));
            }
            WorkloadOp::LogicalTdag { logical_id } => {
                let _ = find_layout(&layouts, *logical_id)?;
                rounds.push(PhysicalRound::bare(RoundKind::MagicTdag, *logical_id));
            }
            WorkloadOp::LogicalCcz { a, b, c } => {
                let _ = find_layout(&layouts, *a)?;
                let _ = find_layout(&layouts, *b)?;
                let _ = find_layout(&layouts, *c)?;
                rounds.push(PhysicalRound {
                    kind: RoundKind::MagicCcz,
                    logical_id: *a,
                    local_before: Vec::new(),
                    entangling: Vec::new(),
                    z_cnot_count: 0,
                    local_mid: Vec::new(),
                    local_after: Vec::new(),
                    terminal: Vec::new(),
                    partner_logical_id: Some(*c),
                    frame_updates: Vec::new(),
                });
            }
        }
    }

    Ok(ExpandedWorkload {
        blocks: layouts,
        rounds,
    })
}

fn find_layout(
    layouts: &[ExpandedBlock],
    id: LogicalQubitId,
) -> Result<&ExpandedBlock, ExpandError> {
    layouts
        .iter()
        .find(|b| b.logical_id == id)
        .ok_or(ExpandError::UnknownLogicalId(id.0))
}

fn expand_block_layout(
    meta: &WorkloadBlock,
    next_atom: &mut u32,
) -> Result<ExpandedBlock, ExpandError> {
    match meta.family {
        SourceFamily::Repetition => expand_repetition_layout(meta, next_atom),
        SourceFamily::Surface => expand_surface_layout(meta, next_atom),
    }
}

/// Alternating `D C D C … D` chain; `N = 2d − 1`.
fn expand_repetition_layout(
    meta: &WorkloadBlock,
    next_atom: &mut u32,
) -> Result<ExpandedBlock, ExpandError> {
    let n = repetition_n(meta.distance)?;
    let first = *next_atom;
    let last = next_atom
        .checked_add(n)
        .ok_or(ExpandError::AtomIdOverflow)?;
    *next_atom = last;

    let atoms: Vec<PhysicalAtomId> = (first..last).map(PhysicalAtomId).collect();
    let coords: Vec<(i32, i32)> = (0..n as i32).map(|i| (i, 0)).collect();
    let mut data_atoms = Vec::with_capacity(meta.distance as usize);
    let mut check_atoms = Vec::with_capacity(meta.distance.saturating_sub(1) as usize);
    for (offset, &atom) in atoms.iter().enumerate() {
        if offset % 2 == 0 {
            data_atoms.push(atom);
        } else {
            check_atoms.push(atom);
        }
    }
    let mut stabilizers = Vec::with_capacity(check_atoms.len());
    for i in 0..check_atoms.len() {
        stabilizers.push(StabilizerDef {
            check: check_atoms[i],
            basis: LogicalBasis::Z,
            data: vec![data_atoms[i], data_atoms[i + 1]],
        });
    }

    Ok(ExpandedBlock {
        logical_id: meta.logical_id,
        family: meta.family,
        code_family: meta.code_family.clone(),
        distance: meta.distance,
        init_basis: meta.init_basis,
        atoms,
        data_atoms,
        check_atoms,
        coords,
        stabilizers,
    })
}

/// Rotated surface layout: `d²` data + `d² − 1` checks (Stim-faithful geometry).
///
/// Public for [`crate::lattice_surgery`] transitional ancilla allocation.
pub(crate) fn expand_surface_layout_for_surgery(
    meta: &WorkloadBlock,
    next_atom: &mut u32,
) -> Result<ExpandedBlock, ExpandError> {
    expand_surface_layout(meta, next_atom)
}

/// Rotated surface layout: `d²` data + `d² − 1` checks (Stim-faithful geometry).
fn expand_surface_layout(
    meta: &WorkloadBlock,
    next_atom: &mut u32,
) -> Result<ExpandedBlock, ExpandError> {
    let n = surface_n(meta.distance)?;
    let d = meta.distance as i32;
    let expected_checks = n.saturating_sub(meta.distance.saturating_mul(meta.distance));

    // Data at odd×odd lattice sites, row-major atom ids.
    let mut data_coord: Vec<((i32, i32), u32)> = Vec::with_capacity((d * d) as usize);
    for r in 0..d {
        for c in 0..d {
            data_coord.push(((2 * c + 1, 2 * r + 1), (r * d + c) as u32));
        }
    }
    let data_at: std::collections::HashMap<(i32, i32), u32> = data_coord.iter().copied().collect();

    // Plaquette centers on even×even sites; smooth (top/bottom) X, rough (L/R) Z.
    let mut plaquettes: Vec<((i32, i32), LogicalBasis, Vec<u32>)> = Vec::new();
    for y in (0..=2 * d).step_by(2) {
        for x in (0..=2 * d).step_by(2) {
            let mut neighbors = Vec::new();
            for (dx, dy) in [(-1, -1), (-1, 1), (1, -1), (1, 1)] {
                if let Some(&idx) = data_at.get(&(x + dx, y + dy)) {
                    neighbors.push(idx);
                }
            }
            if neighbors.len() < 2 {
                continue;
            }
            neighbors.sort_unstable();
            let basis = if (x + y) % 4 == 2 {
                LogicalBasis::X
            } else {
                LogicalBasis::Z
            };
            let on_top = y == 0;
            let on_bot = y == 2 * d;
            let on_left = x == 0;
            let on_right = x == 2 * d;
            if (on_top || on_bot) && basis != LogicalBasis::X {
                continue;
            }
            if (on_left || on_right) && basis != LogicalBasis::Z {
                continue;
            }
            if (on_top || on_bot) && (on_left || on_right) {
                continue;
            }
            plaquettes.push(((x, y), basis, neighbors));
        }
    }
    plaquettes.sort_by_key(|&((x, y), _, _)| (y, x));

    if plaquettes.len() != expected_checks as usize {
        return Err(ExpandError::InvalidSurfaceLayout {
            distance: meta.distance,
            expected: expected_checks,
            got: plaquettes.len(),
        });
    }

    let first = *next_atom;
    let last = next_atom
        .checked_add(n)
        .ok_or(ExpandError::AtomIdOverflow)?;
    *next_atom = last;

    let data_atoms: Vec<PhysicalAtomId> = (0..(d * d) as u32)
        .map(|i| PhysicalAtomId(first + i))
        .collect();
    let check_atoms: Vec<PhysicalAtomId> = (0..plaquettes.len() as u32)
        .map(|i| PhysicalAtomId(first + (d * d) as u32 + i))
        .collect();

    let mut atoms = Vec::with_capacity(n as usize);
    let mut coords = Vec::with_capacity(n as usize);
    for r in 0..d {
        for c in 0..d {
            atoms.push(PhysicalAtomId(first + (r * d + c) as u32));
            coords.push((2 * c + 1, 2 * r + 1));
        }
    }
    let mut stabilizers = Vec::with_capacity(plaquettes.len());
    for (i, &((x, y), basis, ref neighbors)) in plaquettes.iter().enumerate() {
        let check = check_atoms[i];
        atoms.push(check);
        coords.push((x, y));
        stabilizers.push(StabilizerDef {
            check,
            basis,
            data: neighbors
                .iter()
                .map(|&local| PhysicalAtomId(first + local))
                .collect(),
        });
    }

    Ok(ExpandedBlock {
        logical_id: meta.logical_id,
        family: meta.family,
        code_family: meta.code_family.clone(),
        distance: meta.distance,
        init_basis: meta.init_basis,
        atoms,
        data_atoms,
        check_atoms,
        coords,
        stabilizers,
    })
}

/// Kelly-style bit-flip syndrome: each check `C_i` is the CNOT target of
/// neighboring data `D_i` and `D_{i+1}`.
fn repetition_cnot_pattern(layout: &ExpandedBlock) -> Result<Vec<PhysicalCnot>, ExpandError> {
    let d = layout.distance;
    if layout.data_atoms.len() != d as usize
        || layout.check_atoms.len() != d.saturating_sub(1) as usize
    {
        return Err(ExpandError::Qec(QecError::InvalidRepetitionDistance {
            distance: d,
        }));
    }
    let mut cnots = Vec::with_capacity(2 * layout.check_atoms.len());
    for i in 0..layout.check_atoms.len() {
        let check = layout.check_atoms[i];
        let left = layout.data_atoms[i];
        let right = layout.data_atoms[i + 1];
        cnots.push(PhysicalCnot {
            control: left,
            target: check,
        });
        cnots.push(PhysicalCnot {
            control: right,
            target: check,
        });
    }
    Ok(cnots)
}

fn repetition_memory_round(layout: &ExpandedBlock) -> Result<PhysicalRound, ExpandError> {
    let entangling = repetition_cnot_pattern(layout)?;
    let mut terminal = Vec::with_capacity(2 * layout.check_atoms.len());
    for &atom in &layout.check_atoms {
        terminal.push(RoundTerminal::Measure {
            atom,
            basis: LogicalBasis::Z,
        });
    }
    for &atom in &layout.check_atoms {
        terminal.push(RoundTerminal::Reset { atom });
    }
    let z_cnot_count = entangling.len();
    Ok(PhysicalRound {
        kind: RoundKind::MemoryRound,
        logical_id: layout.logical_id,
        local_before: Vec::new(),
        entangling,
        z_cnot_count,
        local_mid: Vec::new(),
        local_after: Vec::new(),
        terminal,
        partner_logical_id: None,
        frame_updates: Vec::new(),
    })
}

/// Full surface stabilizer extraction (Z-then-X). Used for memory rounds and
/// post-merge split restore rounds (kind overridden by the caller).
pub(crate) fn surface_memory_round(layout: &ExpandedBlock) -> PhysicalRound {
    // Z-then-X phases: overlapping X/Z CX in one block does not implement
    // simultaneous stabilizer extraction (Stim needs its 4-layer schedule or
    // a serial Z-then-X split). We take the serial split for all odd d.
    let mut z_cnots = Vec::new();
    let mut x_cnots = Vec::new();
    let mut x_h = Vec::new();

    for stab in &layout.stabilizers {
        match stab.basis {
            LogicalBasis::X => {
                x_h.push(RoundLocalOp::H { atom: stab.check });
                for &data in &stab.data {
                    x_cnots.push(PhysicalCnot {
                        control: stab.check,
                        target: data,
                    });
                }
            }
            LogicalBasis::Z => {
                for &data in &stab.data {
                    z_cnots.push(PhysicalCnot {
                        control: data,
                        target: stab.check,
                    });
                }
            }
        }
    }

    let z_cnot_count = z_cnots.len();
    let mut entangling = z_cnots;
    entangling.extend(x_cnots);

    let mut terminal = Vec::with_capacity(2 * layout.check_atoms.len());
    for &atom in &layout.check_atoms {
        terminal.push(RoundTerminal::Measure {
            atom,
            basis: LogicalBasis::Z,
        });
    }
    for &atom in &layout.check_atoms {
        terminal.push(RoundTerminal::Reset { atom });
    }

    PhysicalRound {
        kind: RoundKind::MemoryRound,
        logical_id: layout.logical_id,
        local_before: Vec::new(),
        entangling,
        z_cnot_count,
        local_mid: x_h.clone(),
        local_after: x_h,
        terminal,
        partner_logical_id: None,
        frame_updates: Vec::new(),
    }
}

fn measure_logical_round(layout: &ExpandedBlock, basis: LogicalBasis) -> PhysicalRound {
    let terminal = layout
        .data_atoms
        .iter()
        .map(|&atom| RoundTerminal::Measure { atom, basis })
        .collect();
    PhysicalRound {
        kind: RoundKind::MeasureLogical,
        logical_id: layout.logical_id,
        local_before: Vec::new(),
        entangling: Vec::new(),
        z_cnot_count: 0,
        local_mid: Vec::new(),
        local_after: Vec::new(),
        terminal,
        partner_logical_id: None,
        frame_updates: Vec::new(),
    }
}

fn validate_round_z_cnot(round: &PhysicalRound) -> Result<(), ExpandError> {
    if round.z_cnot_count > round.entangling.len() {
        return Err(ExpandError::InvalidZCnotCount {
            z_cnot_count: round.z_cnot_count,
            entangling_len: round.entangling.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::WorkloadBuilder;

    fn repetition_d3_two_rounds() -> QecWorkload {
        let mut b = WorkloadBuilder::new();
        b.construct(
            SourceFamily::Repetition,
            3,
            LogicalBasis::Z,
            LogicalQubitId(0),
        )
        .expect("construct");
        b.memory_round(LogicalQubitId(0)).expect("r1");
        b.memory_round(LogicalQubitId(0)).expect("r2");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .expect("mz");
        b.finish()
    }

    fn surface_d3_two_rounds() -> QecWorkload {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .expect("construct");
        b.memory_round(LogicalQubitId(0)).expect("r1");
        b.memory_round(LogicalQubitId(0)).expect("r2");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .expect("mz");
        b.finish()
    }

    #[test]
    fn repetition_d3_layout_is_alternating_five_atoms() {
        let expanded = expand_workload(&repetition_d3_two_rounds()).expect("expand");
        assert_eq!(expanded.blocks.len(), 1);
        let block = &expanded.blocks[0];
        assert_eq!(block.distance, 3);
        assert_eq!(
            block.code_family,
            CodeFamily::RepetitionCodeToy { distance: 3 }
        );
        assert_eq!(
            block.atoms,
            vec![
                PhysicalAtomId(0),
                PhysicalAtomId(1),
                PhysicalAtomId(2),
                PhysicalAtomId(3),
                PhysicalAtomId(4),
            ]
        );
        assert_eq!(
            block.data_atoms,
            vec![PhysicalAtomId(0), PhysicalAtomId(2), PhysicalAtomId(4)]
        );
        assert_eq!(
            block.check_atoms,
            vec![PhysicalAtomId(1), PhysicalAtomId(3)]
        );
        assert_eq!(expanded.physical_atom_count(), 5);
        assert_eq!(block.init_basis, LogicalBasis::Z);
        assert_eq!(block.stabilizers.len(), 2);
    }

    #[test]
    fn repetition_d3_memory_round_cnot_measure_reset_pattern() {
        let expanded = expand_workload(&repetition_d3_two_rounds()).expect("expand");
        assert_eq!(expanded.memory_round_count(), 2);

        let memory: Vec<_> = expanded
            .rounds
            .iter()
            .filter(|r| r.kind == RoundKind::MemoryRound)
            .collect();
        assert_eq!(memory.len(), 2);

        let expected_cnot = vec![
            PhysicalCnot {
                control: PhysicalAtomId(0),
                target: PhysicalAtomId(1),
            },
            PhysicalCnot {
                control: PhysicalAtomId(2),
                target: PhysicalAtomId(1),
            },
            PhysicalCnot {
                control: PhysicalAtomId(2),
                target: PhysicalAtomId(3),
            },
            PhysicalCnot {
                control: PhysicalAtomId(4),
                target: PhysicalAtomId(3),
            },
        ];
        for round in &memory {
            assert!(round.local_before.is_empty());
            assert!(round.local_after.is_empty());
            assert_eq!(round.entangling, expected_cnot);
            assert_eq!(
                round.terminal,
                vec![
                    RoundTerminal::Measure {
                        atom: PhysicalAtomId(1),
                        basis: LogicalBasis::Z,
                    },
                    RoundTerminal::Measure {
                        atom: PhysicalAtomId(3),
                        basis: LogicalBasis::Z,
                    },
                    RoundTerminal::Reset {
                        atom: PhysicalAtomId(1),
                    },
                    RoundTerminal::Reset {
                        atom: PhysicalAtomId(3),
                    },
                ]
            );
        }
    }

    #[test]
    fn repetition_d3_op_order_construct_rounds_measure() {
        let expanded = expand_workload(&repetition_d3_two_rounds()).expect("expand");
        let kinds: Vec<_> = expanded.rounds.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RoundKind::Construct,
                RoundKind::MemoryRound,
                RoundKind::MemoryRound,
                RoundKind::MeasureLogical,
            ]
        );
        let mz = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::MeasureLogical)
            .expect("mz");
        assert!(mz.entangling.is_empty());
        assert_eq!(
            mz.terminal,
            vec![
                RoundTerminal::Measure {
                    atom: PhysicalAtomId(0),
                    basis: LogicalBasis::Z,
                },
                RoundTerminal::Measure {
                    atom: PhysicalAtomId(2),
                    basis: LogicalBasis::Z,
                },
                RoundTerminal::Measure {
                    atom: PhysicalAtomId(4),
                    basis: LogicalBasis::Z,
                },
            ]
        );
    }

    #[test]
    fn surface_d3_layout_is_seventeen_atoms_with_eight_checks() {
        let expanded = expand_workload(&surface_d3_two_rounds()).expect("expand");
        assert_eq!(expanded.physical_atom_count(), 17);
        let block = &expanded.blocks[0];
        assert_eq!(block.family, SourceFamily::Surface);
        assert_eq!(
            block.code_family,
            CodeFamily::SurfaceCodeLike { distance: 3 }
        );
        assert_eq!(block.data_atoms.len(), 9);
        assert_eq!(block.check_atoms.len(), 8);
        assert_eq!(block.stabilizers.len(), 8);
        assert_eq!(
            block.data_atoms,
            (0..9).map(PhysicalAtomId).collect::<Vec<_>>()
        );
        assert_eq!(
            block.check_atoms,
            (9..17).map(PhysicalAtomId).collect::<Vec<_>>()
        );
        let x_checks: Vec<_> = block
            .stabilizers
            .iter()
            .filter(|s| s.basis == LogicalBasis::X)
            .map(|s| s.check.0)
            .collect();
        let z_checks: Vec<_> = block
            .stabilizers
            .iter()
            .filter(|s| s.basis == LogicalBasis::Z)
            .map(|s| s.check.0)
            .collect();
        assert_eq!(x_checks, vec![9, 11, 14, 16]);
        assert_eq!(z_checks, vec![10, 12, 13, 15]);
        assert_eq!(block.coords[0], (1, 1));
        assert_eq!(block.coords[8], (5, 5));
        assert_eq!(block.coords[9], (2, 0)); // top X
    }

    #[test]
    fn surface_d3_memory_round_cnot_h_measure_reset_pattern() {
        let expanded = expand_workload(&surface_d3_two_rounds()).expect("expand");
        assert_eq!(expanded.memory_round_count(), 2);
        let memory = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::MemoryRound)
            .expect("memory");

        let h_atoms: Vec<_> = memory
            .local_mid
            .iter()
            .map(|op| match op {
                RoundLocalOp::H { atom } => atom.0,
            })
            .collect();
        assert_eq!(h_atoms, vec![9, 11, 14, 16]);
        assert!(memory.local_before.is_empty());
        assert_eq!(memory.local_mid, memory.local_after);
        assert_eq!(memory.entangling.len(), 24);
        assert_eq!(memory.z_cnot_count, 12);

        let expected_cnot = vec![
            // Z10: [0,1,3,4]
            PhysicalCnot {
                control: PhysicalAtomId(0),
                target: PhysicalAtomId(10),
            },
            PhysicalCnot {
                control: PhysicalAtomId(1),
                target: PhysicalAtomId(10),
            },
            PhysicalCnot {
                control: PhysicalAtomId(3),
                target: PhysicalAtomId(10),
            },
            PhysicalCnot {
                control: PhysicalAtomId(4),
                target: PhysicalAtomId(10),
            },
            // Z12: [2,5]
            PhysicalCnot {
                control: PhysicalAtomId(2),
                target: PhysicalAtomId(12),
            },
            PhysicalCnot {
                control: PhysicalAtomId(5),
                target: PhysicalAtomId(12),
            },
            // Z13: [3,6]
            PhysicalCnot {
                control: PhysicalAtomId(3),
                target: PhysicalAtomId(13),
            },
            PhysicalCnot {
                control: PhysicalAtomId(6),
                target: PhysicalAtomId(13),
            },
            // Z15: [4,5,7,8]
            PhysicalCnot {
                control: PhysicalAtomId(4),
                target: PhysicalAtomId(15),
            },
            PhysicalCnot {
                control: PhysicalAtomId(5),
                target: PhysicalAtomId(15),
            },
            PhysicalCnot {
                control: PhysicalAtomId(7),
                target: PhysicalAtomId(15),
            },
            PhysicalCnot {
                control: PhysicalAtomId(8),
                target: PhysicalAtomId(15),
            },
            // X9: [0,1]
            PhysicalCnot {
                control: PhysicalAtomId(9),
                target: PhysicalAtomId(0),
            },
            PhysicalCnot {
                control: PhysicalAtomId(9),
                target: PhysicalAtomId(1),
            },
            // X11: [1,2,4,5]
            PhysicalCnot {
                control: PhysicalAtomId(11),
                target: PhysicalAtomId(1),
            },
            PhysicalCnot {
                control: PhysicalAtomId(11),
                target: PhysicalAtomId(2),
            },
            PhysicalCnot {
                control: PhysicalAtomId(11),
                target: PhysicalAtomId(4),
            },
            PhysicalCnot {
                control: PhysicalAtomId(11),
                target: PhysicalAtomId(5),
            },
            // X14: [3,4,6,7]
            PhysicalCnot {
                control: PhysicalAtomId(14),
                target: PhysicalAtomId(3),
            },
            PhysicalCnot {
                control: PhysicalAtomId(14),
                target: PhysicalAtomId(4),
            },
            PhysicalCnot {
                control: PhysicalAtomId(14),
                target: PhysicalAtomId(6),
            },
            PhysicalCnot {
                control: PhysicalAtomId(14),
                target: PhysicalAtomId(7),
            },
            // X16: [7,8]
            PhysicalCnot {
                control: PhysicalAtomId(16),
                target: PhysicalAtomId(7),
            },
            PhysicalCnot {
                control: PhysicalAtomId(16),
                target: PhysicalAtomId(8),
            },
        ];
        assert_eq!(memory.entangling, expected_cnot);

        assert_eq!(memory.terminal.len(), 16); // 8 MZ + 8 Reset
        for (i, atom) in (9..17).enumerate() {
            assert_eq!(
                memory.terminal[i],
                RoundTerminal::Measure {
                    atom: PhysicalAtomId(atom),
                    basis: LogicalBasis::Z,
                }
            );
            assert_eq!(
                memory.terminal[8 + i],
                RoundTerminal::Reset {
                    atom: PhysicalAtomId(atom),
                }
            );
        }
    }

    #[test]
    fn surface_d3_op_order_construct_rounds_measure() {
        let expanded = expand_workload(&surface_d3_two_rounds()).expect("expand");
        let kinds: Vec<_> = expanded.rounds.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RoundKind::Construct,
                RoundKind::MemoryRound,
                RoundKind::MemoryRound,
                RoundKind::MeasureLogical,
            ]
        );
        let mz = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::MeasureLogical)
            .expect("mz");
        assert_eq!(mz.terminal.len(), 9);
        assert!(mz.terminal.iter().all(|t| matches!(
            t,
            RoundTerminal::Measure {
                basis: LogicalBasis::Z,
                ..
            }
        )));
    }

    #[test]
    fn surface_code_x_construct_emits_data_hadamards() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::X, LogicalQubitId(0))
            .expect("construct_x");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::X)
            .expect("mx");
        let expanded = expand_workload(&b.finish()).expect("expand");
        assert_eq!(expanded.blocks[0].init_basis, LogicalBasis::X);
        assert_eq!(expanded.physical_atom_count(), 17);
        let construct = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::Construct)
            .expect("construct");
        let h_atoms: Vec<_> = construct
            .local_before
            .iter()
            .map(|op| match op {
                RoundLocalOp::H { atom } => atom.0,
            })
            .collect();
        assert_eq!(h_atoms, (0..9).collect::<Vec<_>>());
    }

    #[test]
    fn z_cnot_count_over_entangling_is_rejected() {
        let round = PhysicalRound {
            kind: RoundKind::MemoryRound,
            logical_id: LogicalQubitId(0),
            local_before: Vec::new(),
            entangling: vec![PhysicalCnot {
                control: PhysicalAtomId(0),
                target: PhysicalAtomId(1),
            }],
            z_cnot_count: 2,
            local_mid: Vec::new(),
            local_after: Vec::new(),
            terminal: Vec::new(),
            partner_logical_id: None,
            frame_updates: Vec::new(),
        };
        let err = validate_round_z_cnot(&round).expect_err("over");
        assert!(matches!(
            err,
            ExpandError::InvalidZCnotCount {
                z_cnot_count: 2,
                entangling_len: 1
            }
        ));
    }

    /// Pauli-tableau style stabilizer algebra for odd-distance rotated surface.
    #[test]
    fn surface_stabilizers_commute_independent_and_logicals_anticommute_d3_d5() {
        for d in [3u32, 5] {
            let mut b = WorkloadBuilder::new();
            b.construct(SourceFamily::Surface, d, LogicalBasis::Z, LogicalQubitId(0))
                .expect("construct");
            let expanded = expand_workload(&b.finish()).expect("expand");
            let block = &expanded.blocks[0];
            let n_data = block.data_atoms.len();
            let data_index: std::collections::HashMap<u32, usize> = block
                .data_atoms
                .iter()
                .enumerate()
                .map(|(i, a)| (a.0, i))
                .collect();

            // Each stabilizer → (x_bits, z_bits) over data qubits (CSS).
            let mut rows: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
            for stab in &block.stabilizers {
                let mut x = vec![0u8; n_data];
                let mut z = vec![0u8; n_data];
                for data in &stab.data {
                    let i = data_index[&data.0];
                    match stab.basis {
                        LogicalBasis::X => x[i] = 1,
                        LogicalBasis::Z => z[i] = 1,
                    }
                }
                rows.push((x, z));
            }

            // Pairwise commutation: symplectic product zero.
            for i in 0..rows.len() {
                for j in i + 1..rows.len() {
                    let mut syn = 0u8;
                    for k in 0..n_data {
                        syn ^= rows[i].0[k] & rows[j].1[k];
                        syn ^= rows[i].1[k] & rows[j].0[k];
                    }
                    assert_eq!(syn, 0, "d={d}: stabilizers {i} and {j} must commute");
                }
            }

            // GF(2) row independence of the stacked X|Z check matrix.
            let mut matrix: Vec<Vec<u8>> = rows
                .iter()
                .map(|(x, z)| {
                    let mut row = x.clone();
                    row.extend(z);
                    row
                })
                .collect();
            let rank = gf2_rank(&mut matrix);
            assert_eq!(
                rank,
                rows.len(),
                "d={d}: expected {n} independent stabilizers, rank={rank}",
                n = rows.len()
            );

            // Logical X = left column; Logical Z = top row — anticommute (overlap 1).
            let d_usize = d as usize;
            let mut lx = vec![0u8; n_data];
            let mut lz = vec![0u8; n_data];
            for r in 0..d_usize {
                lx[r * d_usize] = 1; // left column
            }
            lz[..d_usize].fill(1); // top row
            let mut syn = 0u8;
            for k in 0..n_data {
                syn ^= lx[k] & lz[k];
            }
            assert_eq!(syn, 1, "d={d}: LX and LZ must anticommute");
        }
    }

    fn gf2_rank(rows: &mut [Vec<u8>]) -> usize {
        if rows.is_empty() {
            return 0;
        }
        let cols = rows[0].len();
        let mut rank = 0;
        let mut row_i = 0;
        for col in 0..cols {
            let pivot = rows
                .iter()
                .enumerate()
                .skip(row_i)
                .find(|(_, row)| row[col] == 1)
                .map(|(r, _)| r);
            let Some(pivot) = pivot else {
                continue;
            };
            rows.swap(row_i, pivot);
            let pivot_row = rows[row_i].clone();
            for (r, row) in rows.iter_mut().enumerate() {
                if r != row_i && row[col] == 1 {
                    for (dst, src) in row.iter_mut().zip(&pivot_row) {
                        *dst ^= *src;
                    }
                }
            }
            rank += 1;
            row_i += 1;
            if row_i >= rows.len() {
                break;
            }
        }
        rank
    }

    #[test]
    fn surface_d5_layout_counts() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 5, LogicalBasis::Z, LogicalQubitId(0))
            .expect("construct");
        let expanded = expand_workload(&b.finish()).expect("expand");
        assert_eq!(expanded.physical_atom_count(), 49);
        assert_eq!(expanded.blocks[0].data_atoms.len(), 25);
        assert_eq!(expanded.blocks[0].check_atoms.len(), 24);
        assert_eq!(expanded.blocks[0].stabilizers.len(), 24);
    }

    fn surface_d3_cx_workload_with_memory() -> QecWorkload {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .expect("c");
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(1))
            .expect("t");
        b.memory_round(LogicalQubitId(0)).expect("mr0");
        b.memory_round(LogicalQubitId(1)).expect("mr1");
        b.logical_cx(LogicalQubitId(0), LogicalQubitId(1))
            .expect("cx");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .expect("mz0");
        b.measure_logical(LogicalQubitId(1), LogicalBasis::Z)
            .expect("mz1");
        b.finish()
    }

    #[test]
    fn logical_cx_d3_expands_to_merge_split_phases() {
        let expanded = expand_workload(&surface_d3_cx_workload_with_memory()).expect("expand");

        assert!(expanded.has_lattice_surgery());
        // control + target + ancilla
        assert_eq!(expanded.blocks.len(), 3);
        // 17 + 17 + 17 data/check + 2*3 seam atoms attached to ancilla
        assert_eq!(expanded.physical_atom_count(), 17 + 17 + 17 + 6);
        assert!(expanded.memory_round_count() >= 2);

        let kinds: Vec<_> = expanded.rounds.iter().map(|r| r.kind).collect();
        assert!(
            kinds.contains(&RoundKind::Merge(MergeBoundary::Rough)),
            "missing rough merge: {kinds:?}"
        );
        assert!(
            kinds.contains(&RoundKind::Merge(MergeBoundary::Smooth)),
            "missing smooth merge: {kinds:?}"
        );
        assert!(
            kinds.contains(&RoundKind::Split(MergeBoundary::Rough)),
            "missing rough split: {kinds:?}"
        );
        assert!(
            kinds.contains(&RoundKind::Split(MergeBoundary::Smooth)),
            "missing smooth split: {kinds:?}"
        );
        assert!(kinds.contains(&RoundKind::MeasureAncilla));
        assert!(kinds.contains(&RoundKind::FrameUpdate));

        let rough = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::Merge(MergeBoundary::Rough))
            .expect("rough");
        assert_eq!(rough.entangling.len(), 6); // d=3 pairs × 2 CNOTs
        assert_eq!(rough.z_cnot_count, 6);
        assert!(!rough.terminal.is_empty());

        let smooth = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::Merge(MergeBoundary::Smooth))
            .expect("smooth");
        assert_eq!(smooth.entangling.len(), 6);
        assert_eq!(smooth.z_cnot_count, 0);
        assert!(!smooth.local_mid.is_empty());

        // Split rounds re-measure the seam (not Wait-only placeholders).
        let splits: Vec<_> = expanded
            .rounds
            .iter()
            .filter(|r| matches!(r.kind, RoundKind::Split(_)))
            .collect();
        assert_eq!(
            splits.len(),
            2,
            "expected rough+smooth split, got {}",
            splits.len()
        );
        for s in &splits {
            assert!(
                !s.entangling.is_empty() && !s.terminal.is_empty(),
                "split must emit seam re-measurements: {s:?}"
            );
        }

        let frame = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::FrameUpdate)
            .expect("frame");
        assert!(!frame.frame_updates.is_empty());
        for upd in &frame.frame_updates {
            assert!(
                !upd.condition_atoms.is_empty(),
                "byproduct must be outcome-conditioned: {upd:?}"
            );
        }

        let mz_anc = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::MeasureAncilla)
            .expect("ancilla mz");
        assert_eq!(
            mz_anc.terminal.len(),
            3,
            "ancilla logical Z is top-row (d atoms), not all data"
        );

        // L-shaped: control left of ancilla (L/R rough seam); target below ancilla
        // (top/bottom smooth seam).
        let c = &expanded.blocks[0];
        let t = &expanded.blocks[1];
        let a = expanded
            .blocks
            .iter()
            .find(|b| b.logical_id == LogicalQubitId(2))
            .expect("ancilla");
        let c_max_x = c.coords.iter().map(|(x, _)| *x).max().unwrap();
        let a_min_x = a
            .coords
            .iter()
            .take(a.data_atoms.len())
            .map(|(x, _)| *x)
            .min()
            .unwrap();
        let a_max_y = a
            .coords
            .iter()
            .take(a.data_atoms.len())
            .map(|(_, y)| *y)
            .max()
            .unwrap();
        let t_min_y = t.coords.iter().map(|(_, y)| *y).min().unwrap();
        let t_min_x = t.coords.iter().map(|(x, _)| *x).min().unwrap();
        assert!(
            c_max_x < a_min_x,
            "control|ancilla L/R seam: {c_max_x} < {a_min_x}"
        );
        assert!(
            a_max_y < t_min_y,
            "ancilla/target top/bottom seam: {a_max_y} < {t_min_y}"
        );
        assert!(
            (t_min_x - a_min_x).abs() <= 2,
            "target should sit under ancilla (aligned x), got a_min_x={a_min_x} t_min_x={t_min_x}"
        );

        // Seam checks must carry StabilizerDefs (no orphan check atoms).
        let seam_checks: Vec<_> = a
            .check_atoms
            .iter()
            .filter(|c| !a.stabilizers.iter().any(|s| s.check == **c))
            .collect();
        assert!(
            seam_checks.is_empty(),
            "orphan seam checks without StabilizerDef: {seam_checks:?}"
        );
        assert!(
            a.stabilizers
                .iter()
                .any(|s| s.basis == LogicalBasis::Z && s.data.len() == 2),
            "missing rough ZZ seam stabilizer"
        );
        assert!(
            a.stabilizers
                .iter()
                .any(|s| s.basis == LogicalBasis::X && s.data.len() == 2),
            "missing smooth XX seam stabilizer"
        );
    }

    #[test]
    fn repetition_construct_rejects_non_z_init_basis() {
        let workload = QecWorkload {
            blocks: vec![WorkloadBlock {
                logical_id: LogicalQubitId(0),
                family: SourceFamily::Repetition,
                distance: 3,
                init_basis: LogicalBasis::X,
                code_family: CodeFamily::RepetitionCodeToy { distance: 3 },
            }],
            ops: vec![WorkloadOp::Construct {
                family: SourceFamily::Repetition,
                distance: 3,
                basis: LogicalBasis::X,
                logical_id: LogicalQubitId(0),
            }],
        };
        let err = expand_workload(&workload).expect_err("non-z");
        assert!(matches!(
            err,
            ExpandError::NonZInitBasis {
                logical_id: 0,
                basis: "x"
            }
        ));
    }

    #[test]
    fn logical_t_expands_to_magic_t_round() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.memory_round(LogicalQubitId(0)).unwrap();
        b.logical_t(LogicalQubitId(0)).unwrap();
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .unwrap();
        let expanded = expand_workload(&b.finish()).expect("expand");
        assert_eq!(expanded.t_count(), 1);
        assert_eq!(expanded.tdag_count(), 0);
        assert_eq!(expanded.ccz_count(), 0);
        assert_eq!(expanded.magic_state_demand(), 1);
        assert!(expanded.rounds.iter().any(|r| r.kind == RoundKind::MagicT));
    }

    #[test]
    fn logical_ccz_expands_to_magic_ccz_round() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(1))
            .unwrap();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(2))
            .unwrap();
        b.memory_round(LogicalQubitId(0)).unwrap();
        b.logical_ccz(LogicalQubitId(0), LogicalQubitId(1), LogicalQubitId(2))
            .unwrap();
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .unwrap();
        b.measure_logical(LogicalQubitId(1), LogicalBasis::Z)
            .unwrap();
        b.measure_logical(LogicalQubitId(2), LogicalBasis::Z)
            .unwrap();
        let expanded = expand_workload(&b.finish()).expect("expand");
        assert_eq!(expanded.ccz_count(), 1);
        assert_eq!(expanded.magic_state_demand(), 1);
        assert!(
            expanded
                .rounds
                .iter()
                .any(|r| r.kind == RoundKind::MagicCcz)
        );
    }

    #[test]
    fn logical_t_tdag_ccz_counts_track_independently() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .unwrap();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(1))
            .unwrap();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(2))
            .unwrap();
        b.logical_t(LogicalQubitId(0)).unwrap();
        b.logical_tdag(LogicalQubitId(1)).unwrap();
        b.logical_ccz(LogicalQubitId(0), LogicalQubitId(1), LogicalQubitId(2))
            .unwrap();
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .unwrap();
        b.measure_logical(LogicalQubitId(1), LogicalBasis::Z)
            .unwrap();
        b.measure_logical(LogicalQubitId(2), LogicalBasis::Z)
            .unwrap();
        let expanded = expand_workload(&b.finish()).expect("expand");
        assert_eq!(expanded.t_count(), 1);
        assert_eq!(expanded.tdag_count(), 1);
        assert_eq!(expanded.ccz_count(), 1);
        assert_eq!(expanded.magic_state_demand(), 3);
    }
}
