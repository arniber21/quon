//! QEC experiment artifact: semantic JSON + structure-level Stim (ADR-0018 / #255).
//!
//! Both artifacts are built from the same [`ExpandedWorkload`] in one pass.
//! The `.stim` file is geometry/detectors/observables only — no physical noise
//! channels (ADR-0024). Python annotates noise from the JSON `error_model`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::expand::{
    ExpandedBlock, ExpandedWorkload, PhysicalAtomId, RoundKind, RoundTerminal,
};
use crate::family::SourceFamily;
use crate::workload::LogicalBasis;

/// Wire schema version for `*.qec.json`.
pub const QEC_EXPERIMENT_SCHEMA_VERSION: u32 = 1;

/// Discriminator for the experiment JSON envelope.
pub const QEC_EXPERIMENT_KIND: &str = "qec_experiment";

/// Snapshot of target physical error rates embedded in experiment JSON.
///
/// Field names match [`backend::NeutralAtomErrorModelSnapshot`] / target wire
/// form (ADR-0017). Required when emitting an experiment — never invented.
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

/// One stabilizer check in the expanded layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StabilizerCheck {
    pub logical_id: u32,
    /// Check ancilla atom id.
    pub check_atom: u32,
    /// Neighboring data atoms whose ZZ parity this check extracts.
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
}

impl ExperimentRoundKind {
    fn from_expand(kind: RoundKind) -> Self {
        match kind {
            RoundKind::Construct => Self::Construct,
            RoundKind::MemoryRound => Self::MemoryRound,
            RoundKind::MeasureLogical => Self::MeasureLogical,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Construct => "construct",
            Self::MemoryRound => "memory_round",
            Self::MeasureLogical => "measure_logical",
        }
    }
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
    pub basis: Option<String>,
}

/// Logical observable (product of physical Pauli measurements).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalObservable {
    pub id: u32,
    pub logical_id: u32,
    pub basis: String,
    pub atoms: Vec<u32>,
}

/// Atom role within a code block (layout order).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomSiteMapping {
    pub atom: u32,
    /// `"data"` or `"check"`.
    pub role: String,
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
        "QEC experiment Stim emit currently supports a single repetition-code block; \
         got {block_count} blocks (surface / multi-block is out of scope for #255)"
    )]
    UnsupportedLayout { block_count: usize },
    #[error(
        "QEC experiment emit requires repetition-code blocks; got family `{family}` \
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
    for block in &expanded.blocks {
        if block.family != SourceFamily::Repetition {
            return Err(ExperimentError::UnsupportedFamily {
                family: block.family.as_str(),
                distance: block.distance,
            });
        }
    }
    // v1 metadata is homogeneous single-block; Stim also requires one block.
    let primary = &expanded.blocks[0];
    if expanded.blocks.len() != 1 {
        return Err(ExperimentError::UnsupportedLayout {
            block_count: expanded.blocks.len(),
        });
    }

    let check_graph = check_graph_from_blocks(&expanded.blocks);
    let measurement_schedule = measurement_schedule_from_expanded(expanded);
    let logical_observables = logical_observables_from_blocks(&expanded.blocks);
    let atom_site_map = atom_site_map_from_blocks(&expanded.blocks);

    Ok(QecExperiment {
        schema_version: QEC_EXPERIMENT_SCHEMA_VERSION,
        kind: QEC_EXPERIMENT_KIND.to_string(),
        family: primary.family.as_str().to_string(),
        code_family: primary.code_family.as_report_str().to_string(),
        distance: primary.distance,
        rounds: expanded.memory_round_count() as u32,
        logical_ids: expanded.blocks.iter().map(|b| b.logical_id.0).collect(),
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

/// Attach durable Wait barrier cycles (in order) onto memory-round `na_refs`.
///
/// `barrier_cycles` is the list of schedule layer cycle indices of Wait
/// barriers after memory rounds, in program order.
pub fn attach_barrier_cycles(na_refs: &mut [NaScheduleRef], barrier_cycles: &[u32]) {
    let mut bi = 0usize;
    for r in na_refs.iter_mut() {
        if r.kind == ExperimentRoundKind::MemoryRound {
            if let Some(&cycle) = barrier_cycles.get(bi) {
                r.barrier_cycle = Some(cycle);
            }
            bi += 1;
        }
    }
}

/// Emit a structure-only Stim circuit for repetition-code memory (no noise).
///
/// Layout matches Kelly-style alternating `D C D C … D` with CNOT(data→check)
/// syndrome extraction. Detectors compare consecutive check measurements;
/// final data measurements close the last detectors and form the Z observable.
pub fn emit_stim_structure(expanded: &ExpandedWorkload) -> Result<String, ExperimentError> {
    if expanded.blocks.len() != 1 {
        return Err(ExperimentError::UnsupportedLayout {
            block_count: expanded.blocks.len(),
        });
    }
    let block = &expanded.blocks[0];
    if block.family != SourceFamily::Repetition {
        return Err(ExperimentError::UnsupportedFamily {
            family: block.family.as_str(),
            distance: block.distance,
        });
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

    let mut out = String::new();
    out.push_str(&format!(
        "# Quon QEC experiment — structure only (no noise; ADR-0024)\n\
         # family=repetition distance={} memory_rounds={}\n",
        block.distance,
        memory_rounds.len()
    ));

    for (i, atom) in block.atoms.iter().enumerate() {
        out.push_str(&format!("QUBIT_COORDS({i}, 0) {}\n", atom.0));
    }

    // Prepare |0…0⟩
    out.push_str("R");
    for atom in &block.atoms {
        out.push_str(&format!(" {}", atom.0));
    }
    out.push_str("\nTICK\n");

    // rec[] stack after each MR of all checks: newest is last check.
    // After round k (1-based), we have k * n_checks check measurements.
    for (round_i, round) in memory_rounds.iter().enumerate() {
        // Entangling CNOTs (control data → target check)
        if !round.entangling.is_empty() {
            out.push_str("CX");
            for cnot in &round.entangling {
                out.push_str(&format!(" {} {}", cnot.control.0, cnot.target.0));
            }
            out.push_str("\nTICK\n");
        }

        // Measure + reset checks (Stim MR)
        out.push_str("MR");
        for term in &round.terminal {
            if let RoundTerminal::Measure { atom, .. } = term {
                out.push_str(&format!(" {}", atom.0));
            }
        }
        out.push('\n');

        // Detectors: first round vs +1 (reset |0|); later rounds vs prior round.
        for c in 0..n_checks {
            // Within this MR, check c is rec[-(n_checks - c)]
            let cur = -(n_checks as i32 - c as i32);
            if round_i == 0 {
                out.push_str(&format!(
                    "DETECTOR({c}, {round_i}) rec[{cur}]\n"
                ));
            } else {
                // Previous round's same check is n_checks measurements earlier.
                let prev = cur - n_checks as i32;
                out.push_str(&format!(
                    "DETECTOR({c}, {round_i}) rec[{cur}] rec[{prev}]\n"
                ));
            }
        }
        out.push_str("TICK\n");
    }

    // Final logical Z measurement on data atoms.
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

    out.push_str("M");
    for atom in &data_atoms {
        out.push_str(&format!(" {}", atom.0));
    }
    out.push('\n');

    // Close detectors: each check's last syndrome vs product of its two data Zs.
    // After M of d data qubits, rec[-d .. -1] are data measurements (layout order).
    // Last check MR is still under the data measurements: check c is at
    // rec[-(d + n_checks - c)].
    let d = data_atoms.len() as i32;
    if !memory_rounds.is_empty() && n_checks > 0 {
        let final_round = memory_rounds.len();
        for (c, stab) in check_graph_stabilizers(block).into_iter().enumerate() {
            let check_rec = -(d + n_checks as i32 - c as i32);
            // Data atom indices in the final M record (layout order of data_atoms).
            let mut parts = format!("DETECTOR({c}, {final_round}) rec[{check_rec}]");
            for data in &stab.data_atoms {
                let pos = data_atoms
                    .iter()
                    .position(|a| a.0 == *data)
                    .ok_or(ExperimentError::MissingDataMeasurement { atom: *data })?;
                let data_rec = -(d - pos as i32);
                parts.push_str(&format!(" rec[{data_rec}]"));
            }
            out.push_str(&parts);
            out.push('\n');
        }
    }

    // Logical Z observable = product of all data Z measurements.
    out.push_str("OBSERVABLE_INCLUDE(0)");
    for i in 0..data_atoms.len() {
        let rec = -(d - i as i32);
        out.push_str(&format!(" rec[{rec}]"));
    }
    out.push('\n');

    Ok(out)
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

fn check_graph_from_blocks(blocks: &[ExpandedBlock]) -> CheckGraph {
    let mut atoms = Vec::new();
    let mut data_atoms = Vec::new();
    let mut check_atoms = Vec::new();
    let mut stabilizers = Vec::new();
    for block in blocks {
        atoms.extend(block.atoms.iter().map(|a| a.0));
        data_atoms.extend(block.data_atoms.iter().map(|a| a.0));
        check_atoms.extend(block.check_atoms.iter().map(|a| a.0));
        stabilizers.extend(check_graph_stabilizers(block));
    }
    CheckGraph {
        atoms,
        data_atoms,
        check_atoms,
        stabilizers,
    }
}

fn check_graph_stabilizers(block: &ExpandedBlock) -> Vec<StabilizerCheck> {
    let mut out = Vec::with_capacity(block.check_atoms.len());
    for i in 0..block.check_atoms.len() {
        out.push(StabilizerCheck {
            logical_id: block.logical_id.0,
            check_atom: block.check_atoms[i].0,
            data_atoms: vec![block.data_atoms[i].0, block.data_atoms[i + 1].0],
        });
    }
    out
}

fn measurement_schedule_from_expanded(expanded: &ExpandedWorkload) -> Vec<MeasurementScheduleEntry> {
    expanded
        .rounds
        .iter()
        .enumerate()
        .map(|(i, round)| {
            let (measured_atoms, basis) = match round.kind {
                RoundKind::Construct => (Vec::new(), None),
                RoundKind::MemoryRound | RoundKind::MeasureLogical => {
                    let mut atoms = Vec::new();
                    let mut basis = None;
                    for term in &round.terminal {
                        if let RoundTerminal::Measure {
                            atom,
                            basis: b,
                        } = term
                        {
                            atoms.push(atom.0);
                            basis = Some(b.as_str().to_string());
                        }
                    }
                    (atoms, basis)
                }
            };
            MeasurementScheduleEntry {
                round_index: i as u32,
                kind: ExperimentRoundKind::from_expand(round.kind),
                logical_id: round.logical_id.0,
                measured_atoms,
                basis,
            }
        })
        .collect()
}

fn logical_observables_from_blocks(blocks: &[ExpandedBlock]) -> Vec<LogicalObservable> {
    blocks
        .iter()
        .enumerate()
        .map(|(i, block)| {
            // Repetition Z-memory: logical Z is the product of data Z operators.
            let basis = match block.init_basis {
                LogicalBasis::Z => "z",
                LogicalBasis::X => "x",
            };
            LogicalObservable {
                id: i as u32,
                logical_id: block.logical_id.0,
                basis: basis.to_string(),
                atoms: block.data_atoms.iter().map(|a| a.0).collect(),
            }
        })
        .collect()
}

fn atom_site_map_from_blocks(blocks: &[ExpandedBlock]) -> Vec<AtomSiteMapping> {
    let mut out = Vec::new();
    for block in blocks {
        for (index_in_block, atom) in block.atoms.iter().enumerate() {
            let role = if index_in_block % 2 == 0 {
                "data"
            } else {
                "check"
            };
            out.push(AtomSiteMapping {
                atom: atom.0,
                role: role.to_string(),
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
        assert_eq!(
            exp.check_graph.stabilizers[0].data_atoms,
            vec![0, 2]
        );
        assert_eq!(exp.logical_observables[0].atoms, vec![0, 2, 4]);
        assert_eq!(exp.na_refs.len(), 4);
        assert_eq!(exp.na_refs[1].kind, ExperimentRoundKind::MemoryRound);

        let json = experiment_to_json(&exp).expect("json");
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
        assert!(stim.contains("CX 0 1 2 1 2 3 4 3"), "{stim}");
        assert!(stim.contains("MR 1 3"), "{stim}");
        assert!(stim.contains("DETECTOR"), "{stim}");
        assert!(stim.contains("OBSERVABLE_INCLUDE(0)"), "{stim}");
        assert!(stim.contains("M 0 2 4"), "{stim}");

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
        attach_barrier_cycles(&mut refs, &[7, 15]);
        assert_eq!(refs[1].barrier_cycle, Some(7));
        assert_eq!(refs[2].barrier_cycle, Some(15));
        assert!(refs[0].barrier_cycle.is_none());
        assert!(refs[3].barrier_cycle.is_none());
    }
}
