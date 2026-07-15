//! QEC experiment artifact: semantic JSON + structure-level Stim (ADR-0018 / #255).
//!
//! Both artifacts are built from the same [`ExpandedWorkload`] in one pass.
//! The `.stim` file is geometry/detectors/observables only — no physical noise
//! channels (ADR-0024). Python annotates noise from the JSON `error_model`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::expand::{
    ExpandedBlock, ExpandedWorkload, PhysicalAtomId, PhysicalCnot, RoundKind, RoundTerminal,
};
use crate::family::SourceFamily;
use crate::workload::LogicalBasis;

/// Wire schema version for `*.qec.json`.
pub const QEC_EXPERIMENT_SCHEMA_VERSION: u32 = 1;

/// Discriminator for the experiment JSON envelope.
pub const QEC_EXPERIMENT_KIND: &str = "qec_experiment";

/// Snapshot of target physical error rates embedded in experiment JSON.
///
/// Canonical serde DTO shared with the neutral-atom target wire form
/// (ADR-0017). Backend re-exports this as `NeutralAtomErrorModelSnapshot`.
/// Required when emitting an experiment — never invented.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorModelSnapshot {
    pub rydberg: f64,
    pub measurement: f64,
    pub reset: f64,
    pub movement: f64,
    pub transfer: f64,
    pub idle_per_us: f64,
}

/// Atom role within a code block (layout order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomRole {
    Data,
    Check,
}

impl AtomRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Check => "check",
        }
    }
}

/// One stabilizer check in the expanded layout.
///
/// `basis` defaults to `z` on deserialize so schema_version 1 consumers from
/// #255 (repetition-only JSON without `basis`) keep loading. Surface emits
/// explicit `"x"` / `"z"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StabilizerCheck {
    pub logical_id: u32,
    /// Check ancilla atom id.
    pub check_atom: u32,
    /// Pauli type of this stabilizer (`x` / `z`).
    #[serde(default)]
    pub basis: LogicalBasis,
    /// Data atoms in the stabilizer support.
    pub data_atoms: Vec<u32>,
}

/// Stabilizer / check graph for the expanded blocks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckGraph {
    pub atoms: Vec<u32>,
    pub data_atoms: Vec<u32>,
    pub check_atoms: Vec<u32>,
    pub stabilizers: Vec<StabilizerCheck>,
}

/// Kind of a scheduled QEC round in experiment metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentRoundKind {
    Construct,
    MemoryRound,
    MeasureLogical,
    MergeRough,
    MergeSmooth,
    SplitRough,
    SplitSmooth,
    MeasureAncilla,
    FrameUpdate,
}

impl ExperimentRoundKind {
    fn from_expand(kind: RoundKind) -> Self {
        match kind {
            RoundKind::Construct => Self::Construct,
            RoundKind::MemoryRound => Self::MemoryRound,
            RoundKind::MeasureLogical => Self::MeasureLogical,
            RoundKind::Merge(crate::expand::MergeBoundary::Rough) => Self::MergeRough,
            RoundKind::Merge(crate::expand::MergeBoundary::Smooth) => Self::MergeSmooth,
            RoundKind::Split(crate::expand::MergeBoundary::Rough) => Self::SplitRough,
            RoundKind::Split(crate::expand::MergeBoundary::Smooth) => Self::SplitSmooth,
            RoundKind::MeasureAncilla => Self::MeasureAncilla,
            RoundKind::FrameUpdate => Self::FrameUpdate,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Construct => "construct",
            Self::MemoryRound => "memory_round",
            Self::MeasureLogical => "measure_logical",
            Self::MergeRough => "merge_rough",
            Self::MergeSmooth => "merge_smooth",
            Self::SplitRough => "split_rough",
            Self::SplitSmooth => "split_smooth",
            Self::MeasureAncilla => "measure_ancilla",
            Self::FrameUpdate => "frame_update",
        }
    }

    /// Rounds that receive a durable Wait `barrier_cycle` in `na_refs`.
    pub fn needs_barrier(self) -> bool {
        matches!(
            self,
            Self::MemoryRound
                | Self::MergeRough
                | Self::MergeSmooth
                | Self::SplitRough
                | Self::SplitSmooth
                | Self::MeasureAncilla
                | Self::FrameUpdate
        )
    }
}

/// Outcome-conditioned Pauli frame byproduct in experiment JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameUpdateEntry {
    pub logical_id: u32,
    pub x: bool,
    pub z: bool,
    pub source: String,
    /// Measurement atoms whose record parity conditions this update (odd → apply).
    pub condition_atoms: Vec<u32>,
}

/// Measurement / terminal events in program-round order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementScheduleEntry {
    pub round_index: u32,
    pub kind: ExperimentRoundKind,
    pub logical_id: u32,
    /// Atoms measured in this round (empty for construct).
    pub measured_atoms: Vec<u32>,
    /// Measurement basis when applicable (`"x"` / `"z"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<LogicalBasis>,
    /// Pauli frame byproducts (populated on `frame_update` rounds).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frame_updates: Vec<FrameUpdateEntry>,
}

/// Logical observable (product of physical Pauli measurements).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalObservable {
    pub id: u32,
    pub logical_id: u32,
    pub basis: LogicalBasis,
    pub atoms: Vec<u32>,
}

/// Atom ↔ site mapping within a code block (layout order).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomSiteMapping {
    pub atom: u32,
    pub role: AtomRole,
    pub logical_id: u32,
    pub index_in_block: u32,
}

/// Reference from a QEC round into the compiled `quantum.na` schedule.
///
/// Cycle fields are optional: filled when the NA schedule from the same compile
/// is available; always present as round structure from the expanded IR.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NaScheduleRef {
    pub round_index: u32,
    pub kind: ExperimentRoundKind,
    pub logical_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_end: Option<u32>,
    /// Cycle of the durable Wait barrier after a memory round, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub barrier_cycle: Option<u32>,
}

/// Versioned semantic QEC experiment JSON (`*.qec.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QecExperiment {
    pub schema_version: u32,
    pub kind: String,
    pub family: String,
    pub code_family: String,
    pub distance: u32,
    /// Number of syndrome-extraction (`memory_round`) cycles.
    pub rounds: u32,
    pub logical_ids: Vec<u32>,
    pub check_graph: CheckGraph,
    pub measurement_schedule: Vec<MeasurementScheduleEntry>,
    pub logical_observables: Vec<LogicalObservable>,
    pub atom_site_map: Vec<AtomSiteMapping>,
    pub error_model: ErrorModelSnapshot,
    pub na_refs: Vec<NaScheduleRef>,
    /// Basename of the sibling structure-level Stim circuit.
    pub stim_file: String,
}

/// Failures building or emitting a QEC experiment.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ExperimentError {
    #[error("QEC experiment emit requires a non-empty expanded workload")]
    EmptyWorkload,
    #[error(
        "QEC experiment Stim emit currently supports a single code block or a \
         lattice-surgery CX gadget; got {block_count} blocks without surgery phases"
    )]
    UnsupportedLayout { block_count: usize },
    #[error(
        "QEC lattice-surgery experiment requires homogeneous surface distance; \
         found mismatched blocks"
    )]
    InhomogeneousLatticeSurgery,
    #[error(
        "QEC experiment emit requires repetition or surface blocks; got family `{family}` \
         distance {distance}"
    )]
    UnsupportedFamily {
        family: &'static str,
        distance: u32,
    },
    #[error("failed to serialize QEC experiment JSON: {0}")]
    Serialize(String),
    #[error("stabilizer data atom {atom} missing from final logical measurement record")]
    MissingDataMeasurement { atom: u32 },
    #[error("expanded block is missing stabilizer definitions")]
    MissingStabilizers,
    #[error(
        "QEC na_refs barrier_cycle: got {got} Wait barrier cycle(s), expected \
         {expected} memory_round(s); refusing unchecked Wait mapping"
    )]
    BarrierCycleMismatch { got: usize, expected: usize },
    #[error(
        "Stim structure emit requires matching measure-logical basis for memory detectors; \
         repetition only supports Z (got `{basis}`)"
    )]
    UnsupportedMeasureBasis { basis: &'static str },
    #[error(
        "z_cnot_count ({z_cnot_count}) exceeds entangling.len() ({entangling_len}); \
         refusing silent clamp"
    )]
    InvalidZCnotCount {
        z_cnot_count: usize,
        entangling_len: usize,
    },
    #[error(
        "surface X-plaquette CNOTs present but local_mid Hadamards missing; \
         refusing vacuous X-check extraction"
    )]
    MissingXCheckHadamards,
    #[error("measure-logical round is missing a uniform measurement basis")]
    MissingMeasureBasis,
    #[error("measure-logical terminals disagree on measurement basis")]
    InconsistentMeasureBasis,
}

/// Build semantic experiment JSON + structure Stim from one expanded IR.
///
/// `stim_file` is the sibling basename recorded in JSON (e.g. `exp.stim`).
/// `na_refs` may be pre-enriched with cycle/barrier indices from the same
/// compile's NA schedule; pass [`na_refs_from_expanded`] for IR-only refs.
pub fn dual_emit(
    expanded: &ExpandedWorkload,
    error_model: ErrorModelSnapshot,
    stim_file: &str,
    na_refs: Vec<NaScheduleRef>,
) -> Result<(QecExperiment, String), ExperimentError> {
    let experiment = build_experiment(expanded, error_model, stim_file, na_refs)?;
    let stim = emit_stim_structure(expanded)?;
    Ok((experiment, stim))
}

/// Serialize [`QecExperiment`] to pretty JSON (trailing newline not added).
pub fn experiment_to_json(experiment: &QecExperiment) -> Result<String, ExperimentError> {
    serde_json::to_string_pretty(experiment).map_err(|e| ExperimentError::Serialize(e.to_string()))
}

/// Build the semantic experiment document from expanded IR + error model.
pub fn build_experiment(
    expanded: &ExpandedWorkload,
    error_model: ErrorModelSnapshot,
    stim_file: &str,
    na_refs: Vec<NaScheduleRef>,
) -> Result<QecExperiment, ExperimentError> {
    if expanded.blocks.is_empty() {
        return Err(ExperimentError::EmptyWorkload);
    }
    let primary = &expanded.blocks[0];
    if expanded.blocks.len() != 1 {
        if !expanded.has_lattice_surgery() {
            return Err(ExperimentError::UnsupportedLayout {
                block_count: expanded.blocks.len(),
            });
        }
        if !expanded
            .blocks
            .iter()
            .all(|b| b.family == SourceFamily::Surface && b.distance == primary.distance)
        {
            return Err(ExperimentError::InhomogeneousLatticeSurgery);
        }
    }
    match primary.family {
        SourceFamily::Repetition | SourceFamily::Surface => {}
    }

    let check_graph = check_graph_from_blocks(&expanded.blocks)?;
    let measurement_schedule = measurement_schedule_from_expanded(expanded);
    let logical_observables = logical_observables_from_expanded(expanded)?;
    let atom_site_map = atom_site_map_from_blocks(&expanded.blocks);

    Ok(QecExperiment {
        schema_version: QEC_EXPERIMENT_SCHEMA_VERSION,
        kind: QEC_EXPERIMENT_KIND.to_string(),
        family: primary.family.as_str().to_string(),
        code_family: primary.code_family.as_report_str().to_string(),
        distance: primary.distance,
        rounds: expanded.memory_round_count() as u32,
        logical_ids: expanded
            .blocks
            .iter()
            // Skip transitional ancilla (highest id added by surgery) in the
            // primary logical_ids list when surgery present? Keep all for map.
            .map(|b| b.logical_id.0)
            .collect(),
        check_graph,
        measurement_schedule,
        logical_observables,
        atom_site_map,
        error_model,
        na_refs,
        stim_file: stim_file.to_string(),
    })
}

/// Round structure refs derived solely from expanded IR (no schedule cycles).
pub fn na_refs_from_expanded(expanded: &ExpandedWorkload) -> Vec<NaScheduleRef> {
    expanded
        .rounds
        .iter()
        .enumerate()
        .map(|(i, round)| NaScheduleRef {
            round_index: i as u32,
            kind: ExperimentRoundKind::from_expand(round.kind),
            logical_id: round.logical_id.0,
            cycle_start: None,
            cycle_end: None,
            barrier_cycle: None,
        })
        .collect()
}

/// Attach durable Wait barrier cycles (in order) onto barrier-bearing `na_refs`.
///
/// Fails closed unless `barrier_cycles.len()` equals the number of
/// barrier-bearing entries in `na_refs` (memory rounds and lattice-surgery
/// merge/split/ancilla-measure phases).
pub fn attach_barrier_cycles(
    na_refs: &mut [NaScheduleRef],
    barrier_cycles: &[u32],
) -> Result<(), ExperimentError> {
    let expected = na_refs.iter().filter(|r| r.kind.needs_barrier()).count();
    if barrier_cycles.len() != expected {
        return Err(ExperimentError::BarrierCycleMismatch {
            got: barrier_cycles.len(),
            expected,
        });
    }
    let mut bi = 0usize;
    for r in na_refs.iter_mut() {
        if r.kind.needs_barrier() {
            r.barrier_cycle = Some(barrier_cycles[bi]);
            bi += 1;
        }
    }
    Ok(())
}

/// Emit a structure-only Stim circuit.
///
/// Single-block memory: repetition/surface as before.
/// Lattice-surgery CX: three-patch gadget with merge/split seam measurements
/// and dual logical observables (ADR-0019).
pub fn emit_stim_structure(expanded: &ExpandedWorkload) -> Result<String, ExperimentError> {
    if expanded.has_lattice_surgery() {
        return emit_stim_lattice_surgery_cx(expanded);
    }
    if expanded.blocks.len() != 1 {
        return Err(ExperimentError::UnsupportedLayout {
            block_count: expanded.blocks.len(),
        });
    }
    emit_stim_single_block_memory(expanded)
}

fn emit_stim_single_block_memory(expanded: &ExpandedWorkload) -> Result<String, ExperimentError> {
    let block = &expanded.blocks[0];
    match block.family {
        SourceFamily::Repetition | SourceFamily::Surface => {}
    }

    let n_checks = block.check_atoms.len();
    let memory_rounds: Vec<_> = expanded
        .rounds
        .iter()
        .filter(|r| r.kind == RoundKind::MemoryRound)
        .collect();
    let measure_logical = expanded
        .rounds
        .iter()
        .find(|r| r.kind == RoundKind::MeasureLogical);
    let measure_basis = measure_logical_basis(measure_logical)?;
    if block.family == SourceFamily::Repetition && measure_basis != LogicalBasis::Z {
        return Err(ExperimentError::UnsupportedMeasureBasis {
            basis: measure_basis.as_str(),
        });
    }

    let mut out = String::new();
    out.push_str(&format!(
        "# Quon QEC experiment — structure only (no noise; ADR-0024)\n\
         # family={} distance={} memory_rounds={} measure_basis={}\n\
         # Note: surface uses serial Z-then-X expand (not Stim 4-layer FT schedule).\n",
        block.family.as_str(),
        block.distance,
        memory_rounds.len(),
        measure_basis.as_str(),
    ));

    for (atom, &(x, y)) in block.atoms.iter().zip(block.coords.iter()) {
        out.push_str(&format!("QUBIT_COORDS({x}, {y}) {}\n", atom.0));
    }

    // Prepare |0…0⟩ then optional data H for X-init / X-memory (|+⟩^n).
    out.push('R');
    for atom in &block.atoms {
        out.push_str(&format!(" {}", atom.0));
    }
    out.push_str("\nTICK\n");

    let construct = expanded
        .rounds
        .iter()
        .find(|r| r.kind == RoundKind::Construct);
    if let Some(c) = construct {
        emit_local_ops(&mut out, &c.local_before);
    } else if block.init_basis == LogicalBasis::X {
        // Fallback if construct round missing locals but metadata says X.
        let prep: Vec<_> = block
            .data_atoms
            .iter()
            .map(|&atom| crate::expand::RoundLocalOp::H { atom })
            .collect();
        emit_local_ops(&mut out, &prep);
    }

    let z_check_indices: Vec<usize> = block
        .stabilizers
        .iter()
        .enumerate()
        .filter(|(_, s)| s.basis == LogicalBasis::Z)
        .map(|(i, _)| i)
        .collect();
    let x_check_indices: Vec<usize> = block
        .stabilizers
        .iter()
        .enumerate()
        .filter(|(_, s)| s.basis == LogicalBasis::X)
        .map(|(i, _)| i)
        .collect();

    // First-round deterministic checks: Z under |0⟩, X under |+⟩ (Stim style).
    let first_round_detectors: Vec<usize> = match measure_basis {
        LogicalBasis::Z => z_check_indices.clone(),
        LogicalBasis::X => x_check_indices.clone(),
    };

    for (round_i, round) in memory_rounds.iter().enumerate() {
        emit_round_body(&mut out, round)?;
        out.push_str("MR");
        for term in &round.terminal {
            if let RoundTerminal::Measure { atom, .. } = term {
                out.push_str(&format!(" {}", atom.0));
            }
        }
        out.push('\n');

        let detector_indices: Vec<usize> = if round_i == 0 {
            first_round_detectors.clone()
        } else {
            (0..n_checks).collect()
        };
        for &c in &detector_indices {
            let cur = -(n_checks as i32 - c as i32);
            if round_i == 0 {
                out.push_str(&format!("DETECTOR({c}, {round_i}) rec[{cur}]\n"));
            } else {
                let prev = cur - n_checks as i32;
                out.push_str(&format!(
                    "DETECTOR({c}, {round_i}) rec[{cur}] rec[{prev}]\n"
                ));
            }
        }
        out.push_str("TICK\n");
    }

    let data_atoms: Vec<PhysicalAtomId> = if let Some(mz) = measure_logical {
        mz.terminal
            .iter()
            .filter_map(|t| match t {
                RoundTerminal::Measure { atom, .. } => Some(*atom),
                _ => None,
            })
            .collect()
    } else {
        block.data_atoms.clone()
    };

    let measure_op = match measure_basis {
        LogicalBasis::Z => "MZ",
        LogicalBasis::X => "MX",
    };
    out.push_str(measure_op);
    for atom in &data_atoms {
        out.push_str(&format!(" {}", atom.0));
    }
    out.push('\n');

    // Close detectors in the measure basis against final data measurements.
    let closing_indices: Vec<usize> = match measure_basis {
        LogicalBasis::Z => z_check_indices,
        LogicalBasis::X => x_check_indices,
    };
    let d = data_atoms.len() as i32;
    if !memory_rounds.is_empty() && n_checks > 0 {
        let final_round = memory_rounds.len();
        for &c in &closing_indices {
            let stab = &block.stabilizers[c];
            let check_rec = -(d + n_checks as i32 - c as i32);
            let mut parts = format!("DETECTOR({c}, {final_round}) rec[{check_rec}]");
            for data in &stab.data {
                let pos = data_atoms
                    .iter()
                    .position(|a| a.0 == data.0)
                    .ok_or(ExperimentError::MissingDataMeasurement { atom: data.0 })?;
                let data_rec = -(d - pos as i32);
                parts.push_str(&format!(" rec[{data_rec}]"));
            }
            out.push_str(&parts);
            out.push('\n');
        }
    }

    out.push_str("OBSERVABLE_INCLUDE(0)");
    let obs_atoms = logical_observable_atoms(block, measure_basis);
    for atom in &obs_atoms {
        let pos = data_atoms
            .iter()
            .position(|a| a.0 == *atom)
            .ok_or(ExperimentError::MissingDataMeasurement { atom: *atom })?;
        let rec = -(d - pos as i32);
        out.push_str(&format!(" rec[{rec}]"));
    }
    out.push('\n');

    Ok(out)
}

/// Structure Stim for the three-patch lattice-surgery CX gadget.
///
/// Horsman merge outcomes are emitted as logical-operator `MPP` measurements
/// (joint ZZ / XX / ancilla Z) so frame-corrected Z observables are
/// sinter-evaluable. Geometric seam CX schedules stay in the expand/NA path;
/// they are not replayed here (they are not homologous to logical ZZ/XX).
///
/// Merge / ancilla outcomes feed [`OBSERVABLE_INCLUDE`] via frame byproducts —
/// they are **not** bare `DETECTOR`s. `DETECTOR`s are emitted only for explicit
/// `memory_round` Z-stabilizer comparisons under codespace prep.
fn emit_stim_lattice_surgery_cx(expanded: &ExpandedWorkload) -> Result<String, ExperimentError> {
    if expanded.blocks.len() < 2 {
        return Err(ExperimentError::UnsupportedLayout {
            block_count: expanded.blocks.len(),
        });
    }
    let distance = expanded.blocks[0].distance;
    if !expanded
        .blocks
        .iter()
        .all(|b| b.family == SourceFamily::Surface && b.distance == distance)
    {
        return Err(ExperimentError::InhomogeneousLatticeSurgery);
    }

    let control = &expanded.blocks[0];
    let target = &expanded.blocks[1];
    let ancilla = expanded
        .blocks
        .iter()
        .find(|b| b.logical_id.0 >= 2)
        .ok_or(ExperimentError::UnsupportedLayout {
            block_count: expanded.blocks.len(),
        })?;

    let mut out = String::new();
    out.push_str(&format!(
        "# Quon QEC experiment — lattice-surgery CX structure (no noise; ADR-0019/0024)\n\
         # family=surface distance={} blocks={} (L-shaped: control|ancilla / target)\n\
         # Merge/ancilla outcomes → OBSERVABLE_INCLUDE via frame; not bare DETECTORs.\n\
         # Stim merges use logical MPP (Horsman); NA schedules geometric seam CXs.\n\
         # Note: simplified merge/split model; not Stim FT-distance claim.\n",
        distance,
        expanded.blocks.len(),
    ));

    for block in &expanded.blocks {
        for (atom, &(x, y)) in block.atoms.iter().zip(block.coords.iter()) {
            out.push_str(&format!("QUBIT_COORDS({x}, {y}) {}\n", atom.0));
        }
    }

    let mut all_atoms: Vec<u32> = expanded
        .blocks
        .iter()
        .flat_map(|b| b.atoms.iter().map(|a| a.0))
        .collect();
    all_atoms.sort_unstable();
    all_atoms.dedup();
    out.push('R');
    for id in &all_atoms {
        out.push_str(&format!(" {id}"));
    }
    out.push_str("\nTICK\n");

    // Absolute record index for logical byproduct handles (synthetic keys).
    let mut byproduct_rec: std::collections::HashMap<&'static str, i32> =
        std::collections::HashMap::new();
    let mut rec_count: i32 = 0;
    let mut frame_updates: Vec<&crate::expand::PauliFrameUpdate> = Vec::new();
    let mut measure_logical_rounds = Vec::new();
    let mut memory_detector_i = 0u32;

    for round in &expanded.rounds {
        match round.kind {
            RoundKind::Construct => {
                emit_local_ops(&mut out, &round.local_before);
            }
            RoundKind::Merge(crate::expand::MergeBoundary::Rough) => {
                // Logical ZZ(control, ancilla) — one record for the joint parity.
                let mut ops = Vec::new();
                for a in logical_observable_atoms(control, LogicalBasis::Z) {
                    ops.push(format!("Z{a}"));
                }
                for a in logical_observable_atoms(ancilla, LogicalBasis::Z) {
                    ops.push(format!("Z{a}"));
                }
                out.push_str("MPP ");
                out.push_str(&ops.join("*"));
                out.push('\n');
                byproduct_rec.insert("rough_merge", rec_count);
                rec_count += 1;
                out.push_str("TICK\n");
            }
            RoundKind::Merge(crate::expand::MergeBoundary::Smooth) => {
                // Logical XX(ancilla, target).
                let mut ops = Vec::new();
                for a in logical_observable_atoms(ancilla, LogicalBasis::X) {
                    ops.push(format!("X{a}"));
                }
                for a in logical_observable_atoms(target, LogicalBasis::X) {
                    ops.push(format!("X{a}"));
                }
                out.push_str("MPP ");
                out.push_str(&ops.join("*"));
                out.push('\n');
                byproduct_rec.insert("smooth_merge", rec_count);
                rec_count += 1;
                out.push_str("TICK\n");
            }
            RoundKind::Split(_) => {
                // NA schedules full stabilizer restore on split; Stim uses logical
                // MPP merges and must not scramble data with mid-protocol MR.
                out.push_str("# split_restore (NA-scheduled; omitted from Stim MPP path)\n");
            }
            RoundKind::MemoryRound => {
                emit_round_body(&mut out, round)?;
                let measured: Vec<u32> = round
                    .terminal
                    .iter()
                    .filter_map(|t| match t {
                        RoundTerminal::Measure { atom, .. } => Some(atom.0),
                        _ => None,
                    })
                    .collect();
                if measured.is_empty() {
                    continue;
                }
                out.push_str("MR");
                for id in &measured {
                    out.push_str(&format!(" {id}"));
                    rec_count += 1;
                }
                out.push('\n');
                if let Some(block) = expanded
                    .blocks
                    .iter()
                    .find(|b| b.logical_id == round.logical_id)
                {
                    let n = measured.len() as i32;
                    for stab in &block.stabilizers {
                        if stab.basis != LogicalBasis::Z {
                            continue;
                        }
                        let Some(pos) = measured.iter().position(|a| *a == stab.check.0) else {
                            continue;
                        };
                        let cur = -(n - pos as i32);
                        out.push_str(&format!(
                            "DETECTOR({memory_detector_i}, 0) rec[{cur}]\n"
                        ));
                        memory_detector_i += 1;
                    }
                }
                out.push_str("TICK\n");
            }
            RoundKind::MeasureAncilla => {
                // Logical Z of ancilla (top-row product) as one MPP record.
                let ops: Vec<String> = logical_observable_atoms(ancilla, LogicalBasis::Z)
                    .into_iter()
                    .map(|a| format!("Z{a}"))
                    .collect();
                out.push_str("MPP ");
                out.push_str(&ops.join("*"));
                out.push('\n');
                byproduct_rec.insert("ancilla_mz", rec_count);
                rec_count += 1;
                out.push_str("TICK\n");
            }
            RoundKind::FrameUpdate => {
                frame_updates.extend(round.frame_updates.iter());
            }
            RoundKind::MeasureLogical => {
                measure_logical_rounds.push(round);
            }
        }
    }

    let mut obs_id = 0u32;
    for round in measure_logical_rounds {
        let data_atoms: Vec<u32> = round
            .terminal
            .iter()
            .filter_map(|t| match t {
                RoundTerminal::Measure { atom, .. } => Some(atom.0),
                _ => None,
            })
            .collect();
        if data_atoms.is_empty() {
            continue;
        }
        out.push_str("MZ");
        for id in &data_atoms {
            out.push_str(&format!(" {id}"));
            rec_count += 1;
        }
        out.push('\n');
        let block = expanded
            .blocks
            .iter()
            .find(|b| b.logical_id == round.logical_id)
            .ok_or(ExperimentError::UnsupportedLayout {
                block_count: expanded.blocks.len(),
            })?;
        let obs_atoms = logical_observable_atoms(block, LogicalBasis::Z);
        let d = data_atoms.len() as i32;
        out.push_str(&format!("OBSERVABLE_INCLUDE({obs_id})"));
        for atom in &obs_atoms {
            let pos = data_atoms
                .iter()
                .position(|a| a == atom)
                .ok_or(ExperimentError::MissingDataMeasurement { atom: *atom })?;
            let rec = -(d - pos as i32);
            out.push_str(&format!(" rec[{rec}]"));
        }
        // Horsman on Z readout: X-frame byproducts flip Z (include m_zz, m_a on
        // target). Z-frame byproducts (smooth/ancilla → Z on control) commute
        // with Z and are listed below as OBSERVABLE_INCLUDE frame witnesses so
        // merge records remain sinter-visible without scrambling Z_c.
        for upd in &frame_updates {
            if upd.logical_id != round.logical_id || !upd.x {
                continue;
            }
            if let Some(&abs) = byproduct_rec.get(upd.source) {
                let rel = abs - rec_count;
                out.push_str(&format!(" rec[{rel}]"));
            }
        }
        out.push('\n');
        obs_id += 1;
    }

    // Z-frame byproducts (smooth/ancilla → Z on control): visible in Stim, but
    // not OBSERVABLE_INCLUDE — they commute with Z readout and are random as
    // standalone bits under Z-basis |00⟩ evaluation.
    for upd in &frame_updates {
        if !upd.z || upd.x {
            continue;
        }
        if let Some(&abs) = byproduct_rec.get(upd.source) {
            if frame_updates
                .iter()
                .take_while(|u| !std::ptr::eq(*u, upd))
                .any(|u| u.z && !u.x && u.source == upd.source)
            {
                continue;
            }
            let rel = abs - rec_count;
            out.push_str(&format!(
                "# frame_z source={} rec[{rel}]\n",
                upd.source
            ));
        }
    }

    if obs_id == 0 {
        for (i, block) in expanded.blocks.iter().take(2).enumerate() {
            let obs_atoms = logical_observable_atoms(block, LogicalBasis::Z);
            out.push_str(&format!("# OBSERVABLE_INCLUDE({i}) atoms"));
            for a in &obs_atoms {
                out.push_str(&format!(" {a}"));
            }
            out.push('\n');
        }
    }

    Ok(out)
}

fn emit_round_body(out: &mut String, round: &crate::expand::PhysicalRound) -> Result<(), ExperimentError> {
    if round.z_cnot_count > round.entangling.len() {
        return Err(ExperimentError::InvalidZCnotCount {
            z_cnot_count: round.z_cnot_count,
            entangling_len: round.entangling.len(),
        });
    }
    emit_local_ops(out, &round.local_before);
    let z_count = round.z_cnot_count;
    let (z_cnots, x_cnots) = round.entangling.split_at(z_count);
    if !x_cnots.is_empty() && round.local_mid.is_empty() {
        return Err(ExperimentError::MissingXCheckHadamards);
    }
    for layer in layer_nonoverlapping_cnots(z_cnots) {
        out.push_str("CX");
        for cnot in layer {
            out.push_str(&format!(" {} {}", cnot.control.0, cnot.target.0));
        }
        out.push_str("\nTICK\n");
    }
    emit_local_ops(out, &round.local_mid);
    for layer in layer_nonoverlapping_cnots(x_cnots) {
        out.push_str("CX");
        for cnot in layer {
            out.push_str(&format!(" {} {}", cnot.control.0, cnot.target.0));
        }
        out.push_str("\nTICK\n");
    }
    emit_local_ops(out, &round.local_after);
    Ok(())
}

fn emit_local_ops(out: &mut String, ops: &[crate::expand::RoundLocalOp]) {
    use crate::expand::RoundLocalOp;
    let mut hs = Vec::new();
    for op in ops {
        match op {
            RoundLocalOp::H { atom } => hs.push(atom.0),
        }
    }
    if hs.is_empty() {
        return;
    }
    out.push('H');
    for id in hs {
        out.push_str(&format!(" {id}"));
    }
    out.push_str("\nTICK\n");
}

fn logical_observable_atoms(block: &ExpandedBlock, basis: LogicalBasis) -> Vec<u32> {
    match block.family {
        SourceFamily::Repetition => block.data_atoms.iter().map(|a| a.0).collect(),
        SourceFamily::Surface => {
            let d = block.distance as usize;
            match basis {
                // Top row of the d×d data grid.
                LogicalBasis::Z => block.data_atoms.iter().take(d).map(|a| a.0).collect(),
                // Left column.
                LogicalBasis::X => (0..d)
                    .map(|r| block.data_atoms[r * d].0)
                    .collect(),
            }
        }
    }
}

/// Sibling `.stim` path for an experiment JSON path (ADR-0018).
///
/// `foo.qec.json` → `foo.stim`; `foo.json` → `foo.stim`.
pub fn sibling_stim_path(json_path: &std::path::Path) -> std::path::PathBuf {
    let file_name = json_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("experiment.qec.json");
    let stem = if let Some(s) = file_name.strip_suffix(".qec.json") {
        s
    } else if let Some(s) = file_name.strip_suffix(".json") {
        s
    } else {
        json_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("experiment")
    };
    json_path.with_file_name(format!("{stem}.stim"))
}

/// Greedy non-overlapping CX layers preserving expand order.
///
/// Stim applies every pair in one `CX` instruction simultaneously, so controls
/// and targets within a layer must be disjoint.
pub fn layer_nonoverlapping_cnots(cnots: &[PhysicalCnot]) -> Vec<Vec<&PhysicalCnot>> {
    let mut remaining: Vec<&PhysicalCnot> = cnots.iter().collect();
    let mut layers = Vec::new();
    while !remaining.is_empty() {
        let mut layer = Vec::new();
        let mut used = HashSet::new();
        let mut next_remaining = Vec::new();
        for cnot in remaining {
            if used.contains(&cnot.control.0) || used.contains(&cnot.target.0) {
                next_remaining.push(cnot);
            } else {
                used.insert(cnot.control.0);
                used.insert(cnot.target.0);
                layer.push(cnot);
            }
        }
        debug_assert!(!layer.is_empty(), "progress on non-empty remaining");
        layers.push(layer);
        remaining = next_remaining;
    }
    layers
}

fn check_graph_from_blocks(blocks: &[ExpandedBlock]) -> Result<CheckGraph, ExperimentError> {
    let mut atoms = Vec::new();
    let mut data_atoms = Vec::new();
    let mut check_atoms = Vec::new();
    let mut stabilizers = Vec::new();
    for block in blocks {
        atoms.extend(block.atoms.iter().map(|a| a.0));
        data_atoms.extend(block.data_atoms.iter().map(|a| a.0));
        check_atoms.extend(block.check_atoms.iter().map(|a| a.0));
        stabilizers.extend(check_graph_stabilizers(block)?);
    }
    Ok(CheckGraph {
        atoms,
        data_atoms,
        check_atoms,
        stabilizers,
    })
}

fn check_graph_stabilizers(block: &ExpandedBlock) -> Result<Vec<StabilizerCheck>, ExperimentError> {
    if block.stabilizers.is_empty() && !block.check_atoms.is_empty() {
        return Err(ExperimentError::MissingStabilizers);
    }
    Ok(block
        .stabilizers
        .iter()
        .map(|s| StabilizerCheck {
            logical_id: block.logical_id.0,
            check_atom: s.check.0,
            basis: s.basis,
            data_atoms: s.data.iter().map(|a| a.0).collect(),
        })
        .collect())
}

fn measurement_schedule_from_expanded(expanded: &ExpandedWorkload) -> Vec<MeasurementScheduleEntry> {
    expanded
        .rounds
        .iter()
        .enumerate()
        .map(|(i, round)| {
            let (measured_atoms, basis) = match round.kind {
                RoundKind::Construct | RoundKind::FrameUpdate => (Vec::new(), None),
                RoundKind::MemoryRound
                | RoundKind::MeasureLogical
                | RoundKind::Merge(_)
                | RoundKind::Split(_)
                | RoundKind::MeasureAncilla => {
                    let mut atoms = Vec::new();
                    let mut basis = None;
                    for term in &round.terminal {
                        if let RoundTerminal::Measure {
                            atom,
                            basis: b,
                        } = term
                        {
                            atoms.push(atom.0);
                            basis = Some(*b);
                        }
                    }
                    (atoms, basis)
                }
            };
            let frame_updates = round
                .frame_updates
                .iter()
                .map(|u| FrameUpdateEntry {
                    logical_id: u.logical_id.0,
                    x: u.x,
                    z: u.z,
                    source: u.source.to_string(),
                    condition_atoms: u.condition_atoms.iter().map(|a| a.0).collect(),
                })
                .collect();
            MeasurementScheduleEntry {
                round_index: i as u32,
                kind: ExperimentRoundKind::from_expand(round.kind),
                logical_id: round.logical_id.0,
                measured_atoms,
                basis,
                frame_updates,
            }
        })
        .collect()
}

fn logical_observables_from_expanded(
    expanded: &ExpandedWorkload,
) -> Result<Vec<LogicalObservable>, ExperimentError> {
    // Prefer explicit measure-logical rounds; for CX gadgets emit one Z
    // observable per non-ancilla surface block (control + target).
    let measure_rounds: Vec<_> = expanded
        .rounds
        .iter()
        .filter(|r| r.kind == RoundKind::MeasureLogical)
        .collect();
    if !measure_rounds.is_empty() {
        let mut out = Vec::new();
        for (i, round) in measure_rounds.iter().enumerate() {
            let basis = measure_logical_basis(Some(round))?;
            let block = expanded
                .blocks
                .iter()
                .find(|b| b.logical_id == round.logical_id)
                .ok_or(ExperimentError::UnsupportedLayout {
                    block_count: expanded.blocks.len(),
                })?;
            out.push(LogicalObservable {
                id: i as u32,
                logical_id: block.logical_id.0,
                basis,
                atoms: logical_observable_atoms(block, basis),
            });
        }
        return Ok(out);
    }

    if expanded.has_lattice_surgery() {
        // Control + target only (skip X-init transitional ancilla).
        let mut out = Vec::new();
        for block in &expanded.blocks {
            if block.init_basis == LogicalBasis::X && block.logical_id.0 >= 2 {
                continue;
            }
            // Ancilla is X-init; user blocks are typically Z-init.
            if block.init_basis == LogicalBasis::X {
                continue;
            }
            out.push(LogicalObservable {
                id: out.len() as u32,
                logical_id: block.logical_id.0,
                basis: LogicalBasis::Z,
                atoms: logical_observable_atoms(block, LogicalBasis::Z),
            });
        }
        return Ok(out);
    }

    let measure_basis = measure_logical_basis(None)?;
    Ok(expanded
        .blocks
        .iter()
        .enumerate()
        .map(|(i, block)| LogicalObservable {
            id: i as u32,
            logical_id: block.logical_id.0,
            basis: measure_basis,
            atoms: logical_observable_atoms(block, measure_basis),
        })
        .collect())
}

fn measure_logical_basis(
    measure_logical: Option<&crate::expand::PhysicalRound>,
) -> Result<LogicalBasis, ExperimentError> {
    let Some(round) = measure_logical else {
        return Err(ExperimentError::MissingMeasureBasis);
    };
    let mut basis = None;
    for term in &round.terminal {
        if let RoundTerminal::Measure { basis: b, .. } = term {
            match basis {
                None => basis = Some(*b),
                Some(prev) if prev != *b => {
                    return Err(ExperimentError::InconsistentMeasureBasis);
                }
                Some(_) => {}
            }
        }
    }
    basis.ok_or(ExperimentError::MissingMeasureBasis)
}

fn atom_site_map_from_blocks(blocks: &[ExpandedBlock]) -> Vec<AtomSiteMapping> {
    let mut out = Vec::new();
    for block in blocks {
        let data_set: HashSet<u32> = block.data_atoms.iter().map(|a| a.0).collect();
        for (index_in_block, atom) in block.atoms.iter().enumerate() {
            let role = if data_set.contains(&atom.0) {
                AtomRole::Data
            } else {
                AtomRole::Check
            };
            out.push(AtomSiteMapping {
                atom: atom.0,
                role,
                logical_id: block.logical_id.0,
                index_in_block: index_in_block as u32,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expand::expand_workload;
    use crate::workload::{LogicalQubitId, WorkloadBuilder};

    fn example_error_model() -> ErrorModelSnapshot {
        ErrorModelSnapshot {
            rydberg: 0.002,
            measurement: 0.003,
            reset: 0.004,
            movement: 0.0005,
            transfer: 0.0007,
            idle_per_us: 2e-9,
        }
    }

    fn repetition_d3_two_rounds() -> ExpandedWorkload {
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
        expand_workload(&b.finish()).expect("expand")
    }

    fn repetition_d3_measure_x() -> ExpandedWorkload {
        let mut b = WorkloadBuilder::new();
        b.construct(
            SourceFamily::Repetition,
            3,
            LogicalBasis::Z,
            LogicalQubitId(0),
        )
        .expect("construct");
        b.memory_round(LogicalQubitId(0)).expect("r1");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::X)
            .expect("mx");
        expand_workload(&b.finish()).expect("expand")
    }

    #[test]
    fn experiment_dto_round_trips_and_embeds_error_model() {
        let expanded = repetition_d3_two_rounds();
        let (exp, _stim) = dual_emit(
            &expanded,
            example_error_model(),
            "repetition_d3.stim",
            na_refs_from_expanded(&expanded),
        )
        .expect("dual_emit");

        assert_eq!(exp.schema_version, 1);
        assert_eq!(exp.kind, QEC_EXPERIMENT_KIND);
        assert_eq!(exp.family, "repetition");
        assert_eq!(exp.code_family, "repetition_code_toy");
        assert_eq!(exp.distance, 3);
        assert_eq!(exp.rounds, 2);
        assert_eq!(exp.logical_ids, vec![0]);
        assert_eq!(exp.error_model.rydberg, 0.002);
        assert_eq!(exp.error_model.measurement, 0.003);
        assert_eq!(exp.stim_file, "repetition_d3.stim");
        assert_eq!(exp.check_graph.check_atoms, vec![1, 3]);
        assert_eq!(exp.check_graph.data_atoms, vec![0, 2, 4]);
        assert_eq!(exp.check_graph.stabilizers.len(), 2);
        assert_eq!(exp.check_graph.stabilizers[0].data_atoms, vec![0, 2]);
        assert_eq!(exp.logical_observables[0].atoms, vec![0, 2, 4]);
        assert_eq!(exp.logical_observables[0].basis, LogicalBasis::Z);
        assert_eq!(exp.check_graph.stabilizers[0].basis, LogicalBasis::Z);
        assert_eq!(exp.atom_site_map[0].role, AtomRole::Data);
        assert_eq!(exp.atom_site_map[1].role, AtomRole::Check);
        assert_eq!(exp.na_refs.len(), 4);
        assert_eq!(exp.na_refs[1].kind, ExperimentRoundKind::MemoryRound);

        let json = experiment_to_json(&exp).expect("json");
        assert!(json.contains("\"basis\": \"z\""), "{json}");
        assert!(json.contains("\"role\": \"data\""), "{json}");
        let back: QecExperiment = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, exp);
    }

    #[test]
    fn experiment_dto_rejects_unknown_fields() {
        let expanded = repetition_d3_two_rounds();
        let (exp, _) = dual_emit(
            &expanded,
            example_error_model(),
            "x.stim",
            na_refs_from_expanded(&expanded),
        )
        .expect("dual_emit");
        let mut value = serde_json::to_value(&exp).expect("value");
        value
            .as_object_mut()
            .expect("object")
            .insert("extra_field".into(), serde_json::json!(1));
        let err = serde_json::from_value::<QecExperiment>(value).expect_err("unknown");
        assert!(
            err.to_string().contains("unknown field") || err.to_string().contains("extra_field"),
            "{err}"
        );
    }

    #[test]
    fn nested_dto_rejects_unknown_fields() {
        let cases: &[(&str, &str)] = &[
            (
                "StabilizerCheck",
                r#"{"logical_id":0,"check_atom":1,"basis":"z","data_atoms":[0,2],"bonus":1}"#,
            ),
            (
                "MeasurementScheduleEntry",
                r#"{"round_index":0,"kind":"construct","logical_id":0,"measured_atoms":[],"extra":true}"#,
            ),
            (
                "LogicalObservable",
                r#"{"id":0,"logical_id":0,"basis":"z","atoms":[0],"noise":1}"#,
            ),
            (
                "AtomSiteMapping",
                r#"{"atom":0,"role":"data","logical_id":0,"index_in_block":0,"zone":1}"#,
            ),
            (
                "NaScheduleRef",
                r#"{"round_index":0,"kind":"memory_round","logical_id":0,"barrier_cycle":3,"hint":0}"#,
            ),
            (
                "CheckGraph",
                r#"{"atoms":[],"data_atoms":[],"check_atoms":[],"stabilizers":[],"layout":1}"#,
            ),
        ];
        for (name, json) in cases {
            let err = match *name {
                "StabilizerCheck" => serde_json::from_str::<StabilizerCheck>(json)
                    .expect_err("unknown")
                    .to_string(),
                "MeasurementScheduleEntry" => {
                    serde_json::from_str::<MeasurementScheduleEntry>(json)
                        .expect_err("unknown")
                        .to_string()
                }
                "LogicalObservable" => serde_json::from_str::<LogicalObservable>(json)
                    .expect_err("unknown")
                    .to_string(),
                "AtomSiteMapping" => serde_json::from_str::<AtomSiteMapping>(json)
                    .expect_err("unknown")
                    .to_string(),
                "NaScheduleRef" => serde_json::from_str::<NaScheduleRef>(json)
                    .expect_err("unknown")
                    .to_string(),
                "CheckGraph" => serde_json::from_str::<CheckGraph>(json)
                    .expect_err("unknown")
                    .to_string(),
                _ => unreachable!(),
            };
            assert!(
                err.contains("unknown field"),
                "{name} should reject unknown fields: {err}"
            );
        }
    }

    #[test]
    fn error_model_snapshot_rejects_unknown_fields() {
        let err = serde_json::from_str::<ErrorModelSnapshot>(
            r#"{"rydberg":0.1,"measurement":0.1,"reset":0.1,"movement":0.1,"transfer":0.1,"idle_per_us":0.1,"bonus":1}"#,
        )
        .expect_err("unknown");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn stim_structure_has_detectors_observables_no_noise() {
        let expanded = repetition_d3_two_rounds();
        let stim = emit_stim_structure(&expanded).expect("stim");

        assert!(stim.contains("QUBIT_COORDS"), "{stim}");
        // Non-overlapping CX layers in expand order (not one packed line).
        assert!(stim.contains("CX 0 1 2 3\nTICK"), "{stim}");
        assert!(stim.contains("CX 2 1 4 3\nTICK"), "{stim}");
        assert!(
            !stim.contains("CX 0 1 2 1 2 3 4 3"),
            "must not pack overlapping CX:\n{stim}"
        );
        assert!(stim.contains("MR 1 3"), "{stim}");
        assert!(stim.contains("DETECTOR"), "{stim}");
        assert!(stim.contains("OBSERVABLE_INCLUDE(0)"), "{stim}");
        assert!(stim.contains("MZ 0 2 4"), "{stim}");

        // Structure only: no Stim noise channels (ADR-0024).
        for line in stim.lines() {
            let op = line.split_whitespace().next().unwrap_or("");
            assert!(
                !matches!(
                    op,
                    "DEPOLARIZE1"
                        | "DEPOLARIZE2"
                        | "X_ERROR"
                        | "Z_ERROR"
                        | "Y_ERROR"
                        | "E"
                        | "ELSE_CORRELATED_ERROR"
                        | "PAULI_CHANNEL_1"
                        | "PAULI_CHANNEL_2"
                        | "HERALDED_ERASE"
                        | "HERALDED_PAULI_CHANNEL_1"
                ),
                "noise op {op} must not appear:\n{stim}"
            );
        }

        // Two memory rounds → detectors at round indices 0 and 1, plus final.
        assert!(stim.contains("DETECTOR(0, 0)"), "{stim}");
        assert!(stim.contains("DETECTOR(0, 1)"), "{stim}");
        assert!(stim.contains("DETECTOR(0, 2)"), "{stim}");
    }

    #[test]
    fn stim_hard_fails_non_z_measure_logical() {
        let expanded = repetition_d3_measure_x();
        let err = emit_stim_structure(&expanded).expect_err("non-Z");
        assert!(
            matches!(err, ExperimentError::UnsupportedMeasureBasis { basis: "x" }),
            "{err:?}"
        );
    }

    #[test]
    fn observables_use_measure_basis_not_init_basis() {
        let expanded = repetition_d3_measure_x();
        // JSON build still records measure basis even though Stim hard-fails.
        let exp = build_experiment(
            &expanded,
            example_error_model(),
            "x.stim",
            na_refs_from_expanded(&expanded),
        )
        .expect("build");
        assert_eq!(exp.logical_observables[0].basis, LogicalBasis::X);
        // init was Z
        assert_eq!(expanded.blocks[0].init_basis, LogicalBasis::Z);
    }

    #[test]
    fn dual_emit_shares_ir_counts() {
        let expanded = repetition_d3_two_rounds();
        let (exp, stim) = dual_emit(
            &expanded,
            example_error_model(),
            "out.stim",
            na_refs_from_expanded(&expanded),
        )
        .expect("dual");
        assert_eq!(exp.rounds, 2);
        assert_eq!(stim.matches("MR 1 3").count(), 2);
        assert_eq!(exp.atom_site_map.len(), 5);
    }

    #[test]
    fn sibling_stim_path_strips_qec_json() {
        let p = std::path::Path::new("/tmp/rep_d3.qec.json");
        assert_eq!(
            sibling_stim_path(p),
            std::path::PathBuf::from("/tmp/rep_d3.stim")
        );
        let p2 = std::path::Path::new("exp.json");
        assert_eq!(
            sibling_stim_path(p2),
            std::path::PathBuf::from("exp.stim")
        );
    }

    #[test]
    fn attach_barrier_cycles_fills_memory_rounds() {
        let expanded = repetition_d3_two_rounds();
        let mut refs = na_refs_from_expanded(&expanded);
        attach_barrier_cycles(&mut refs, &[7, 15]).expect("match");
        assert_eq!(refs[1].barrier_cycle, Some(7));
        assert_eq!(refs[2].barrier_cycle, Some(15));
        assert!(refs[0].barrier_cycle.is_none());
        assert!(refs[3].barrier_cycle.is_none());
    }

    #[test]
    fn attach_barrier_cycles_fails_closed_on_count_mismatch() {
        let expanded = repetition_d3_two_rounds();
        let mut refs = na_refs_from_expanded(&expanded);
        let err = attach_barrier_cycles(&mut refs, &[7]).expect_err("mismatch");
        assert_eq!(
            err,
            ExperimentError::BarrierCycleMismatch {
                got: 1,
                expected: 2
            }
        );
        let err = attach_barrier_cycles(&mut refs, &[1, 2, 3]).expect_err("too many");
        assert_eq!(
            err,
            ExperimentError::BarrierCycleMismatch {
                got: 3,
                expected: 2
            }
        );
    }

    #[test]
    fn check_graph_stabilizers_fail_when_missing() {
        let mut expanded = repetition_d3_two_rounds();
        expanded.blocks[0].stabilizers.clear();
        let err = check_graph_stabilizers(&expanded.blocks[0]).expect_err("missing");
        assert!(matches!(err, ExperimentError::MissingStabilizers), "{err:?}");
    }

    #[test]
    fn surface_d3_dual_emit_json_and_stim() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .expect("construct");
        b.memory_round(LogicalQubitId(0)).expect("r1");
        b.memory_round(LogicalQubitId(0)).expect("r2");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .expect("mz");
        let expanded = expand_workload(&b.finish()).expect("expand");

        let (exp, stim) = dual_emit(
            &expanded,
            example_error_model(),
            "surface_d3.stim",
            na_refs_from_expanded(&expanded),
        )
        .expect("dual_emit");

        assert_eq!(exp.family, "surface");
        assert_eq!(exp.code_family, "surface_code_like");
        assert_eq!(exp.distance, 3);
        assert_eq!(exp.rounds, 2);
        assert_eq!(exp.check_graph.atoms.len(), 17);
        assert_eq!(exp.check_graph.check_atoms.len(), 8);
        assert_eq!(exp.check_graph.stabilizers.len(), 8);
        assert_eq!(
            exp.check_graph
                .stabilizers
                .iter()
                .filter(|s| s.basis == LogicalBasis::X)
                .count(),
            4
        );
        assert_eq!(exp.logical_observables[0].atoms, vec![0, 1, 2]);
        assert_eq!(exp.atom_site_map.len(), 17);
        assert_eq!(exp.atom_site_map[0].role, AtomRole::Data);
        assert_eq!(exp.atom_site_map[9].role, AtomRole::Check);

        assert!(stim.contains("family=surface"), "{stim}");
        assert!(stim.contains("H 9 11 14 16"), "{stim}");
        assert!(stim.contains("MR 9 10 11 12 13 14 15 16"), "{stim}");
        assert!(stim.contains("MZ 0 1 2 3 4 5 6 7 8"), "{stim}");
        assert!(stim.contains("OBSERVABLE_INCLUDE(0)"), "{stim}");
        assert!(stim.contains("DETECTOR"), "{stim}");
        // First-round detectors only on Z checks (indices 1,3,4,6).
        assert!(stim.contains("DETECTOR(1, 0)"), "{stim}");
        assert!(!stim.contains("DETECTOR(0, 0)"), "{stim}");
        for line in stim.lines() {
            let op = line.split_whitespace().next().unwrap_or("");
            assert!(
                !matches!(
                    op,
                    "DEPOLARIZE1"
                        | "DEPOLARIZE2"
                        | "X_ERROR"
                        | "Z_ERROR"
                        | "Y_ERROR"
                        | "E"
                        | "ELSE_CORRELATED_ERROR"
                        | "PAULI_CHANNEL_1"
                        | "PAULI_CHANNEL_2"
                ),
                "noise op {op} must not appear:\n{stim}"
            );
        }
    }

    #[test]
    fn surface_x_memory_stim_rotated_memory_x_style() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::X, LogicalQubitId(0))
            .expect("construct");
        b.memory_round(LogicalQubitId(0)).expect("r1");
        b.memory_round(LogicalQubitId(0)).expect("r2");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::X)
            .expect("mx");
        let expanded = expand_workload(&b.finish()).expect("expand");
        let stim = emit_stim_structure(&expanded).expect("stim");

        // Data H prep for |+⟩ (atoms 0..8).
        assert!(stim.contains("H 0 1 2 3 4 5 6 7 8"), "{stim}");
        assert!(stim.contains("H 9 11 14 16"), "{stim}");
        assert!(stim.contains("MX 0 1 2 3 4 5 6 7 8"), "{stim}");
        // First-round detectors only on X checks (indices 0,2,5,7).
        assert!(stim.contains("DETECTOR(0, 0)"), "{stim}");
        assert!(!stim.contains("DETECTOR(1, 0)"), "{stim}");
        // Left-column observable.
        assert!(stim.contains("OBSERVABLE_INCLUDE(0)"), "{stim}");
        let exp = build_experiment(
            &expanded,
            example_error_model(),
            "sx.stim",
            na_refs_from_expanded(&expanded),
        )
        .expect("build");
        assert_eq!(exp.logical_observables[0].atoms, vec![0, 3, 6]);
        assert_eq!(exp.logical_observables[0].basis, LogicalBasis::X);
    }

    #[test]
    fn stim_rejects_x_cnots_without_mid_hadamards() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .expect("construct");
        b.memory_round(LogicalQubitId(0)).expect("r1");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .expect("mz");
        let mut expanded = expand_workload(&b.finish()).expect("expand");
        for round in &mut expanded.rounds {
            if round.kind == RoundKind::MemoryRound {
                round.local_mid.clear();
                round.local_after.clear();
            }
        }
        let err = emit_stim_structure(&expanded).expect_err("missing H");
        assert!(
            matches!(err, ExperimentError::MissingXCheckHadamards),
            "{err:?}"
        );
    }

    #[test]
    fn stim_rejects_z_cnot_count_overflow() {
        let mut b = WorkloadBuilder::new();
        b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
            .expect("construct");
        b.memory_round(LogicalQubitId(0)).expect("r1");
        b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
            .expect("mz");
        let mut expanded = expand_workload(&b.finish()).expect("expand");
        for round in &mut expanded.rounds {
            if round.kind == RoundKind::MemoryRound {
                round.z_cnot_count = round.entangling.len() + 1;
            }
        }
        let err = emit_stim_structure(&expanded).expect_err("overflow");
        assert!(matches!(err, ExperimentError::InvalidZCnotCount { .. }), "{err:?}");
    }

    #[test]
    fn stabilizer_check_basis_defaults_to_z_for_255_consumers() {
        let check: StabilizerCheck = serde_json::from_str(
            r#"{"logical_id":0,"check_atom":1,"data_atoms":[0,2]}"#,
        )
        .expect("default basis");
        assert_eq!(check.basis, LogicalBasis::Z);
    }

    #[test]
    fn cx_layers_are_non_overlapping_in_expand_order() {
        let expanded = repetition_d3_two_rounds();
        let memory = expanded
            .rounds
            .iter()
            .find(|r| r.kind == RoundKind::MemoryRound)
            .expect("memory");
        let layers = layer_nonoverlapping_cnots(&memory.entangling);
        assert_eq!(layers.len(), 2);
        assert_eq!(
            layers[0]
                .iter()
                .map(|c| (c.control.0, c.target.0))
                .collect::<Vec<_>>(),
            vec![(0, 1), (2, 3)]
        );
        assert_eq!(
            layers[1]
                .iter()
                .map(|c| (c.control.0, c.target.0))
                .collect::<Vec<_>>(),
            vec![(2, 1), (4, 3)]
        );
        for layer in &layers {
            let mut used = HashSet::new();
            for cnot in layer {
                assert!(used.insert(cnot.control.0));
                assert!(used.insert(cnot.target.0));
            }
        }
    }

    fn surface_d3_cx_expanded() -> ExpandedWorkload {
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
        expand_workload(&b.finish()).expect("expand")
    }

    #[test]
    fn lattice_surgery_cx_d3_dual_emit_json_and_stim() {
        let expanded = surface_d3_cx_expanded();
        assert!(expanded.has_lattice_surgery());

        let (exp, stim) = dual_emit(
            &expanded,
            example_error_model(),
            "cx_d3.stim",
            na_refs_from_expanded(&expanded),
        )
        .expect("dual");

        assert_eq!(exp.family, "surface");
        assert_eq!(exp.distance, 3);
        assert!(exp.rounds >= 2, "expected memory rounds around CX");
        assert!(exp.logical_ids.contains(&0));
        assert!(exp.logical_ids.contains(&1));
        assert!(
            exp.na_refs
                .iter()
                .any(|r| r.kind == ExperimentRoundKind::MergeRough),
            "expected merge_rough na_ref"
        );
        assert!(
            exp.na_refs
                .iter()
                .any(|r| r.kind == ExperimentRoundKind::FrameUpdate),
            "expected frame_update na_ref"
        );
        let frame_sched = exp
            .measurement_schedule
            .iter()
            .find(|e| e.kind == ExperimentRoundKind::FrameUpdate)
            .expect("frame_update in measurement_schedule");
        assert!(
            !frame_sched.frame_updates.is_empty(),
            "frame updates must be visible in JSON schedule"
        );
        assert!(
            frame_sched
                .frame_updates
                .iter()
                .all(|u| !u.condition_atoms.is_empty()),
            "frame updates must be outcome-conditioned"
        );
        assert!(exp.logical_observables.len() >= 2, "{:?}", exp.logical_observables);

        assert!(stim.contains("lattice-surgery CX"), "{stim}");
        assert!(stim.contains("L-shaped"), "{stim}");
        assert!(stim.contains("OBSERVABLE_INCLUDE(0)"), "{stim}");
        assert!(stim.contains("OBSERVABLE_INCLUDE(1)"), "{stim}");
        assert!(stim.contains("CX "), "{stim}");
        assert!(!stim.contains("DEPOLARIZE"), "{stim}");
        assert!(!stim.contains("X_ERROR"), "{stim}");
        assert!(stim.contains("MPP "), "logical merge MPP missing: {stim}");
        // Target Z observable must fold rough+ancilla byproduct records.
        let obs1 = stim
            .lines()
            .find(|l| l.starts_with("OBSERVABLE_INCLUDE(1)"))
            .expect("obs1");
        let obs1_recs = obs1.matches("rec[").count();
        assert!(
            obs1_recs >= 5,
            "obs1 must include logical Z atoms plus frame byproduct recs, got {obs1_recs}: {obs1}"
        );
        assert!(
            stim.contains("# frame_z source=smooth_merge"),
            "smooth Z-frame must be visible in Stim: {stim}"
        );
    }

    #[test]
    fn lattice_surgery_stim_omits_bare_merge_detectors() {
        let expanded = surface_d3_cx_expanded();
        let stim = emit_stim_structure(&expanded).expect("stim");
        assert!(
            stim.contains("Merge/ancilla outcomes → OBSERVABLE_INCLUDE"),
            "{stim}"
        );
        assert!(stim.contains("MPP "), "{stim}");
        // Merge outcomes are MPP records, never single-rec DETECTORs on seam MRs.
        for line in stim.lines() {
            if line.starts_with("MPP ") {
                // next non-comment lines should not be DETECTOR tying that MPP alone
                // (policy: byproducts go to OBSERVABLE_INCLUDE).
            }
        }
        // Regression: old bug emitted DETECTOR per seam measure (2*d + 1 = 7+)
        // in addition to memory checks. Memory-only path has 2 rounds × 4 Z checks = 8.
        let detector_lines: Vec<_> = stim
            .lines()
            .filter(|l| l.starts_with("DETECTOR"))
            .collect();
        assert_eq!(
            detector_lines.len(),
            8,
            "expected only memory-round Z detectors, got {detector_lines:?}"
        );

        // Regression: a bare DETECTOR on a random |+⟩-basis MPP (merge-like)
        // must fire — the reason merge outcomes cannot be wired as DETECTORs.
        let py = std::process::Command::new("python3")
            .arg("-c")
            .arg(
                r#"
import stim
# Minimal merge-like random parity with a bare DETECTOR (old bug shape).
c = stim.Circuit()
c.append("R", [0, 1])
c.append("H", [1])
c.append("MPP", [stim.target_z(0), stim.target_combiner(), stim.target_z(1)])
c.append("DETECTOR", [stim.target_rec(-1)])
dets = c.compile_detector_sampler(seed=0).sample(shots=32)
assert dets.any(), "bare DETECTOR on random MPP must fire"
print("ok faulty fires")
"#,
            )
            .output();
        if let Ok(output) = py {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !(stderr.contains("ModuleNotFoundError") || stderr.contains("No module named 'stim'"))
            {
                assert!(
                    output.status.success(),
                    "faulty-detector regression failed: {}",
                    stderr
                );
            }
        }
    }

    #[test]
    fn lattice_surgery_stim_noiseless_codespace_and_observables() {
        let expanded = surface_d3_cx_expanded();
        let stim = emit_stim_structure(&expanded).expect("stim");
        let dir = std::env::temp_dir().join(format!(
            "quon-qec-250-stim-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmpdir");
        let stim_path = dir.join("cx_d3.stim");
        std::fs::write(&stim_path, &stim).expect("write stim");

        let py = std::process::Command::new("python3")
            .arg("-c")
            .arg(format!(
                r#"
import stim
import numpy as np
c = stim.Circuit.from_file({stim_path:?})
assert c.num_observables >= 2, c.num_observables
# Noiseless: detectors (memory Z checks) must be deterministic 0; observables
# for |00⟩→CX→|00⟩ with X-frame corrections on target must be 0.
sampler = c.compile_detector_sampler(seed=0)
dets, obs = sampler.sample(shots=64, separate_observables=True)
assert dets.shape[0] == 64
if dets.size:
    assert not dets.any(), f"detector fired under noiseless codespace: {{dets.sum()}}"
assert not obs.any(), f"observables not deterministic 0 for |00>: {{obs.sum()}}"

# Regression: if merge outcomes were bare DETECTORs expecting 0, injecting a
# synthetic circuit that measures random bits as detectors would fail the check
# above. Also assert OBSERVABLE_INCLUDE(1) references more records than d=3 data.
obs1 = [line for line in str(c).splitlines() if line.startswith("OBSERVABLE_INCLUDE(1)")]
assert obs1 and obs1[0].count("rec[") > 3, obs1
print(f"ok dets={{c.num_detectors}} obs={{c.num_observables}}")
"#
            ))
            .output();
        match py {
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("ModuleNotFoundError") || stderr.contains("No module named 'stim'")
                {
                    eprintln!("skip stim correctness: stim not installed ({stderr})");
                } else {
                    assert!(
                        output.status.success(),
                        "stim correctness failed: status={} stderr={} stdout={}",
                        output.status,
                        stderr,
                        String::from_utf8_lossy(&output.stdout)
                    );
                }
            }
            Err(e) => eprintln!("skip stim correctness: python unavailable ({e})"),
        }
    }
}
