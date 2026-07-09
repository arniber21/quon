// Target descriptor DTO and conversion — see issue #3, SPEC.md §8.3 and
// ADR-0009.
//
// The loader accepts both the legacy flat fixed-connectivity descriptor and the
// neutral-atom reconfigurable descriptor. Conversion into `BackendTarget`
// validates architecture-specific invariants and keeps only `id` shared at the
// top level.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::error::BackendError;
use crate::gates;
use crate::keys;
use crate::target::{
    AodMovement, AodMovementModel, AodSpeedModel, AodSpeedModelKind, BackendTarget,
    ConnectivityGraph, FixedTarget, NeutralAtomCostModel, NeutralAtomFidelity, NeutralAtomGrid,
    NeutralAtomTarget, NeutralAtomTiming, NeutralAtomZone, NoiseModel, RydbergInteraction,
    ZoneKind,
};

/// Top-level target JSON. The fixed descriptor remains backward-compatible:
/// legacy files without `kind` deserialize as [`TargetDescriptor::Fixed`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TargetDescriptor {
    Fixed(FixedTargetDescriptor),
    NeutralAtomReconfigurable(NeutralAtomTargetDescriptor),
}

/// The fixed-connectivity gate-model JSON shape. Fields without
/// `#[serde(default)]` are required, so serde names missing required fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedTargetDescriptor {
    #[serde(default)]
    pub kind: FixedTargetKindDescriptor,
    pub id: String,
    pub num_qubits: usize,
    pub topology: TopologyDescriptor,
    pub native_gates: Vec<String>,
    #[serde(default)]
    pub noise: NoiseDescriptor,
    pub meas_latency_us: f64,
    pub supports_mid_circuit_meas: bool,
    pub supports_feed_forward: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixedTargetKindDescriptor {
    #[default]
    Fixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyDescriptor {
    pub edges: Vec<(usize, usize)>,
}

/// Noise as it appears in fixed-target JSON: gate → qubit-string → value
/// because JSON object keys are strings. Every field defaults to empty.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseDescriptor {
    #[serde(default)]
    pub single_qubit_fidelity: HashMap<String, HashMap<String, f64>>,
    #[serde(default)]
    pub two_qubit_fidelity: HashMap<String, HashMap<String, f64>>,
    #[serde(default)]
    pub t1_us: HashMap<String, f64>,
    #[serde(default)]
    pub t2_us: HashMap<String, f64>,
    #[serde(default)]
    pub readout_error: HashMap<String, f64>,
}

impl NoiseDescriptor {
    fn into_model(self, num_qubits: usize) -> Result<NoiseModel, BackendError> {
        let mut model = NoiseModel::default();

        for (gate, per_qubit) in self.single_qubit_fidelity {
            for (q_key, fid) in per_qubit {
                let q = keys::decode_qubit(&q_key, num_qubits)?;
                model.single_qubit_fidelity.insert((gate.clone(), q), fid);
            }
        }
        for (gate, per_pair) in self.two_qubit_fidelity {
            for (pair_key, fid) in per_pair {
                let (u, v) = keys::decode_pair(&pair_key, num_qubits)?;
                model.two_qubit_fidelity.insert((gate.clone(), u, v), fid);
            }
        }
        for (q_key, t1) in self.t1_us {
            model
                .t1_us
                .insert(keys::decode_qubit(&q_key, num_qubits)?, t1);
        }
        for (q_key, t2) in self.t2_us {
            model
                .t2_us
                .insert(keys::decode_qubit(&q_key, num_qubits)?, t2);
        }
        for (q_key, err) in self.readout_error {
            model
                .readout_error
                .insert(keys::decode_qubit(&q_key, num_qubits)?, err);
        }
        Ok(model)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralAtomTargetDescriptor {
    pub id: String,
    pub kind: NeutralAtomTargetKindDescriptor,
    pub grid: NeutralAtomGridDescriptor,
    pub zones: Vec<NeutralAtomZoneDescriptor>,
    pub movement: AodMovementDescriptor,
    pub interaction: RydbergInteractionDescriptor,
    pub native_gates: Vec<String>,
    pub timing: NeutralAtomTimingDescriptor,
    pub fidelity: NeutralAtomFidelityDescriptor,
    pub cost_model: NeutralAtomCostModelDescriptor,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeutralAtomTargetKindDescriptor {
    NeutralAtomReconfigurable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralAtomGridDescriptor {
    pub width_um: f64,
    pub height_um: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralAtomZoneDescriptor {
    pub zone_id: u32,
    pub kind: ZoneKindDescriptor,
    pub rows: i64,
    pub cols: i64,
    pub origin_um: [f64; 2],
    pub site_pitch_um: [f64; 2],
    #[serde(default)]
    pub pair_gap_um: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneKindDescriptor {
    Storage,
    Entanglement,
    Readout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AodMovementDescriptor {
    pub model: AodMovementModelDescriptor,
    pub aod_rows: i64,
    pub aod_cols: i64,
    pub num_aods: i64,
    pub min_row_col_separation_um: f64,
    pub speed_model: AodSpeedModelDescriptor,
    pub trap_transfer_us: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AodMovementModelDescriptor {
    AodRowColumnCoupled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AodSpeedModelDescriptor {
    pub kind: AodSpeedModelKindDescriptor,
    pub acceleration_m_s2: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AodSpeedModelKindDescriptor {
    Sqrt,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RydbergInteractionDescriptor {
    pub rydberg_range_um: f64,
    pub min_rydberg_spacing_um: f64,
    pub max_parallel_entangling_pairs: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralAtomTimingDescriptor {
    pub cz_us: f64,
    pub single_qubit_us: f64,
    pub measurement_us: f64,
    pub reset_us: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralAtomFidelityDescriptor {
    pub cz: f64,
    pub single_qubit: f64,
    pub atom_transfer: f64,
    pub coherence_time_us: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralAtomCostModelDescriptor {
    pub rydberg_stage_weight: f64,
    pub movement_time_weight: f64,
    pub trap_transfer_weight: f64,
    pub idle_time_weight: f64,
}

impl TryFrom<TargetDescriptor> for BackendTarget {
    type Error = BackendError;

    fn try_from(descriptor: TargetDescriptor) -> Result<Self, Self::Error> {
        match descriptor {
            TargetDescriptor::Fixed(d) => fixed_target_from_descriptor(d),
            TargetDescriptor::NeutralAtomReconfigurable(d) => neutral_atom_from_descriptor(d),
        }
    }
}

impl BackendTarget {
    /// Project back to the JSON wire form. Fixed targets drop decomposition
    /// closures; neutral-atom targets preserve their fully serializable payload.
    pub fn to_descriptor(&self) -> TargetDescriptor {
        match &self.kind {
            crate::target::TargetKind::Fixed(target) => {
                TargetDescriptor::Fixed(fixed_target_to_descriptor(&self.id, target))
            }
            crate::target::TargetKind::NeutralAtomReconfigurable(target) => {
                TargetDescriptor::NeutralAtomReconfigurable(neutral_atom_to_descriptor(
                    &self.id, target,
                ))
            }
        }
    }
}

fn fixed_target_from_descriptor(d: FixedTargetDescriptor) -> Result<BackendTarget, BackendError> {
    let topology = ConnectivityGraph::try_from_edges(d.num_qubits, d.topology.edges)?;
    let native_gates = d
        .native_gates
        .iter()
        .map(|name| gates::native_gate(name))
        .collect::<Result<Vec<_>, _>>()?;
    let noise = d.noise.into_model(d.num_qubits)?;
    Ok(BackendTarget::fixed(
        d.id,
        FixedTarget {
            num_qubits: d.num_qubits,
            topology,
            native_gates,
            noise,
            meas_latency_us: d.meas_latency_us,
            supports_mid_circuit_meas: d.supports_mid_circuit_meas,
            supports_feed_forward: d.supports_feed_forward,
        },
    ))
}

fn fixed_target_to_descriptor(id: &str, target: &FixedTarget) -> FixedTargetDescriptor {
    let mut noise = NoiseDescriptor::default();
    for ((gate, q), fid) in &target.noise.single_qubit_fidelity {
        noise
            .single_qubit_fidelity
            .entry(gate.clone())
            .or_default()
            .insert(keys::encode_qubit(*q), *fid);
    }
    for ((gate, u, v), fid) in &target.noise.two_qubit_fidelity {
        noise
            .two_qubit_fidelity
            .entry(gate.clone())
            .or_default()
            .insert(keys::encode_pair(*u, *v), *fid);
    }
    noise.t1_us = target
        .noise
        .t1_us
        .iter()
        .map(|(q, t)| (keys::encode_qubit(*q), *t))
        .collect();
    noise.t2_us = target
        .noise
        .t2_us
        .iter()
        .map(|(q, t)| (keys::encode_qubit(*q), *t))
        .collect();
    noise.readout_error = target
        .noise
        .readout_error
        .iter()
        .map(|(q, e)| (keys::encode_qubit(*q), *e))
        .collect();

    FixedTargetDescriptor {
        kind: FixedTargetKindDescriptor::Fixed,
        id: id.to_owned(),
        num_qubits: target.num_qubits,
        topology: TopologyDescriptor {
            edges: target.topology.edges.clone(),
        },
        native_gates: target.native_gates.iter().map(|g| g.name.clone()).collect(),
        noise,
        meas_latency_us: target.meas_latency_us,
        supports_mid_circuit_meas: target.supports_mid_circuit_meas,
        supports_feed_forward: target.supports_feed_forward,
    }
}

fn neutral_atom_from_descriptor(
    d: NeutralAtomTargetDescriptor,
) -> Result<BackendTarget, BackendError> {
    let grid = NeutralAtomGrid {
        width_um: positive_f64("grid.width_um", d.grid.width_um)?,
        height_um: positive_f64("grid.height_um", d.grid.height_um)?,
    };

    let zones = d
        .zones
        .into_iter()
        .map(zone_from_descriptor)
        .collect::<Result<Vec<_>, _>>()?;
    validate_zones(grid, &zones)?;

    let movement = AodMovement {
        model: match d.movement.model {
            AodMovementModelDescriptor::AodRowColumnCoupled => AodMovementModel::RowColumnCoupled,
        },
        aod_rows: positive_u32("movement.aod_rows", d.movement.aod_rows)?,
        aod_cols: positive_u32("movement.aod_cols", d.movement.aod_cols)?,
        num_aods: positive_u32("movement.num_aods", d.movement.num_aods)?,
        min_row_col_separation_um: positive_f64(
            "movement.min_row_col_separation_um",
            d.movement.min_row_col_separation_um,
        )?,
        speed_model: AodSpeedModel {
            kind: match d.movement.speed_model.kind {
                AodSpeedModelKindDescriptor::Sqrt => AodSpeedModelKind::Sqrt,
            },
            acceleration_m_s2: positive_f64(
                "movement.speed_model.acceleration_m_s2",
                d.movement.speed_model.acceleration_m_s2,
            )?,
        },
        trap_transfer_us: positive_f64("movement.trap_transfer_us", d.movement.trap_transfer_us)?,
    };

    let max_parallel_entangling_pairs = positive_u32(
        "interaction.max_parallel_entangling_pairs",
        d.interaction.max_parallel_entangling_pairs,
    )?;
    let interaction = RydbergInteraction {
        rydberg_range_um: positive_f64(
            "interaction.rydberg_range_um",
            d.interaction.rydberg_range_um,
        )?,
        min_rydberg_spacing_um: positive_f64(
            "interaction.min_rydberg_spacing_um",
            d.interaction.min_rydberg_spacing_um,
        )?,
        max_parallel_entangling_pairs,
    };

    let target = NeutralAtomTarget {
        grid,
        zones,
        movement,
        interaction,
        native_gates: non_empty_native_gates(d.native_gates)?,
        timing: NeutralAtomTiming {
            cz_us: positive_f64("timing.cz_us", d.timing.cz_us)?,
            single_qubit_us: positive_f64("timing.single_qubit_us", d.timing.single_qubit_us)?,
            measurement_us: positive_f64("timing.measurement_us", d.timing.measurement_us)?,
            reset_us: positive_f64("timing.reset_us", d.timing.reset_us)?,
        },
        fidelity: NeutralAtomFidelity {
            cz: probability("fidelity.cz", d.fidelity.cz)?,
            single_qubit: probability("fidelity.single_qubit", d.fidelity.single_qubit)?,
            atom_transfer: probability("fidelity.atom_transfer", d.fidelity.atom_transfer)?,
            coherence_time_us: positive_f64(
                "fidelity.coherence_time_us",
                d.fidelity.coherence_time_us,
            )?,
        },
        cost_model: NeutralAtomCostModel {
            rydberg_stage_weight: non_negative_f64(
                "cost_model.rydberg_stage_weight",
                d.cost_model.rydberg_stage_weight,
            )?,
            movement_time_weight: non_negative_f64(
                "cost_model.movement_time_weight",
                d.cost_model.movement_time_weight,
            )?,
            trap_transfer_weight: non_negative_f64(
                "cost_model.trap_transfer_weight",
                d.cost_model.trap_transfer_weight,
            )?,
            idle_time_weight: non_negative_f64(
                "cost_model.idle_time_weight",
                d.cost_model.idle_time_weight,
            )?,
        },
    };

    let entanglement_capacity = target.zone_capacity(ZoneKind::Entanglement);
    if u64::from(max_parallel_entangling_pairs) > entanglement_capacity {
        return invalid_config(format!(
            "interaction.max_parallel_entangling_pairs ({max_parallel_entangling_pairs}) exceeds entanglement zone capacity ({entanglement_capacity})"
        ));
    }

    Ok(BackendTarget::neutral_atom_reconfigurable(d.id, target))
}

fn zone_from_descriptor(d: NeutralAtomZoneDescriptor) -> Result<NeutralAtomZone, BackendError> {
    let kind = match d.kind {
        ZoneKindDescriptor::Storage => ZoneKind::Storage,
        ZoneKindDescriptor::Entanglement => ZoneKind::Entanglement,
        ZoneKindDescriptor::Readout => ZoneKind::Readout,
    };
    let pair_gap_um = match (kind, d.pair_gap_um) {
        (ZoneKind::Entanglement, Some(gap)) => Some(positive_f64("zones[].pair_gap_um", gap)?),
        (ZoneKind::Entanglement, None) => {
            return invalid_config("entanglement zones require pair_gap_um");
        }
        (_, Some(_)) => {
            return invalid_config("pair_gap_um is only valid for entanglement zones");
        }
        (_, None) => None,
    };

    Ok(NeutralAtomZone {
        zone_id: d.zone_id,
        kind,
        rows: positive_u32("zones[].rows", d.rows)?,
        cols: positive_u32("zones[].cols", d.cols)?,
        origin_um: (
            non_negative_f64("zones[].origin_um[0]", d.origin_um[0])?,
            non_negative_f64("zones[].origin_um[1]", d.origin_um[1])?,
        ),
        site_pitch_um: (
            positive_f64("zones[].site_pitch_um[0]", d.site_pitch_um[0])?,
            positive_f64("zones[].site_pitch_um[1]", d.site_pitch_um[1])?,
        ),
        pair_gap_um,
    })
}

fn validate_zones(grid: NeutralAtomGrid, zones: &[NeutralAtomZone]) -> Result<(), BackendError> {
    if zones.is_empty() {
        return invalid_config("neutral-atom targets require at least one zone");
    }

    let mut ids = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    for zone in zones {
        if !ids.insert(zone.zone_id) {
            return invalid_config(format!("duplicate zone_id {}", zone.zone_id));
        }
        kinds.insert(zone.kind);

        let (_, _, x_max, y_max) = zone.bounds_um();
        if x_max > grid.width_um || y_max > grid.height_um {
            return invalid_config(format!(
                "zone {} exceeds grid bounds ({}, {}) um",
                zone.zone_id, grid.width_um, grid.height_um
            ));
        }
    }

    for required in [ZoneKind::Storage, ZoneKind::Entanglement, ZoneKind::Readout] {
        if !kinds.contains(&required) {
            return invalid_config(format!("neutral-atom targets require a {required:?} zone"));
        }
    }

    for (i, lhs) in zones.iter().enumerate() {
        for rhs in zones.iter().skip(i + 1) {
            if zones_overlap(lhs, rhs) {
                return invalid_config(format!(
                    "zones {} and {} overlap",
                    lhs.zone_id, rhs.zone_id
                ));
            }
        }
    }

    Ok(())
}

fn zones_overlap(lhs: &NeutralAtomZone, rhs: &NeutralAtomZone) -> bool {
    let (ax0, ay0, ax1, ay1) = lhs.bounds_um();
    let (bx0, by0, bx1, by1) = rhs.bounds_um();
    ax0 < bx1 && bx0 < ax1 && ay0 < by1 && by0 < ay1
}

fn neutral_atom_to_descriptor(id: &str, target: &NeutralAtomTarget) -> NeutralAtomTargetDescriptor {
    NeutralAtomTargetDescriptor {
        id: id.to_owned(),
        kind: NeutralAtomTargetKindDescriptor::NeutralAtomReconfigurable,
        grid: NeutralAtomGridDescriptor {
            width_um: target.grid.width_um,
            height_um: target.grid.height_um,
        },
        zones: target
            .zones
            .iter()
            .map(|zone| NeutralAtomZoneDescriptor {
                zone_id: zone.zone_id,
                kind: match zone.kind {
                    ZoneKind::Storage => ZoneKindDescriptor::Storage,
                    ZoneKind::Entanglement => ZoneKindDescriptor::Entanglement,
                    ZoneKind::Readout => ZoneKindDescriptor::Readout,
                },
                rows: i64::from(zone.rows),
                cols: i64::from(zone.cols),
                origin_um: [zone.origin_um.0, zone.origin_um.1],
                site_pitch_um: [zone.site_pitch_um.0, zone.site_pitch_um.1],
                pair_gap_um: zone.pair_gap_um,
            })
            .collect(),
        movement: AodMovementDescriptor {
            model: AodMovementModelDescriptor::AodRowColumnCoupled,
            aod_rows: i64::from(target.movement.aod_rows),
            aod_cols: i64::from(target.movement.aod_cols),
            num_aods: i64::from(target.movement.num_aods),
            min_row_col_separation_um: target.movement.min_row_col_separation_um,
            speed_model: AodSpeedModelDescriptor {
                kind: AodSpeedModelKindDescriptor::Sqrt,
                acceleration_m_s2: target.movement.speed_model.acceleration_m_s2,
            },
            trap_transfer_us: target.movement.trap_transfer_us,
        },
        interaction: RydbergInteractionDescriptor {
            rydberg_range_um: target.interaction.rydberg_range_um,
            min_rydberg_spacing_um: target.interaction.min_rydberg_spacing_um,
            max_parallel_entangling_pairs: i64::from(
                target.interaction.max_parallel_entangling_pairs,
            ),
        },
        native_gates: target.native_gates.clone(),
        timing: NeutralAtomTimingDescriptor {
            cz_us: target.timing.cz_us,
            single_qubit_us: target.timing.single_qubit_us,
            measurement_us: target.timing.measurement_us,
            reset_us: target.timing.reset_us,
        },
        fidelity: NeutralAtomFidelityDescriptor {
            cz: target.fidelity.cz,
            single_qubit: target.fidelity.single_qubit,
            atom_transfer: target.fidelity.atom_transfer,
            coherence_time_us: target.fidelity.coherence_time_us,
        },
        cost_model: NeutralAtomCostModelDescriptor {
            rydberg_stage_weight: target.cost_model.rydberg_stage_weight,
            movement_time_weight: target.cost_model.movement_time_weight,
            trap_transfer_weight: target.cost_model.trap_transfer_weight,
            idle_time_weight: target.cost_model.idle_time_weight,
        },
    }
}

fn non_empty_native_gates(gates: Vec<String>) -> Result<Vec<String>, BackendError> {
    if gates.is_empty() {
        invalid_config("neutral-atom targets require at least one native gate")
    } else {
        Ok(gates)
    }
}

fn positive_u32(field: &str, value: i64) -> Result<u32, BackendError> {
    if value <= 0 {
        return invalid_config(format!("{field} must be positive, got {value}"));
    }
    u32::try_from(value)
        .map_err(|_| BackendError::InvalidTargetConfig(format!("{field} is too large: {value}")))
}

fn positive_f64(field: &str, value: f64) -> Result<f64, BackendError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        invalid_config(format!("{field} must be positive and finite, got {value}"))
    }
}

fn non_negative_f64(field: &str, value: f64) -> Result<f64, BackendError> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        invalid_config(format!(
            "{field} must be non-negative and finite, got {value}"
        ))
    }
}

#[allow(clippy::manual_range_contains)] // Flux treats RangeInclusive construction as possibly panicking.
fn probability(field: &str, value: f64) -> Result<f64, BackendError> {
    if value.is_finite() && value >= 0.0 && value <= 1.0 {
        Ok(value)
    } else {
        invalid_config(format!(
            "{field} must be a probability in [0, 1], got {value}"
        ))
    }
}

fn invalid_config<T>(message: impl Into<String>) -> Result<T, BackendError> {
    Err(BackendError::InvalidTargetConfig(message.into()))
}
