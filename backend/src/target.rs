// BackendTarget and supporting types — see issue #3, SPEC.md §8.1.
//
// The in-memory domain types here mirror SPEC.md §8.1 exactly, including the
// non-serializable `decompose` closure on `NativeGate`. Serialization lives in
// `crate::descriptor` (the §8.3 JSON wire format) with a `TryFrom` conversion.

#[cfg(feature = "flux")]
use flux_rs::attrs::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Distance sentinel for an unreachable pair in [`ConnectivityGraph::dist`].
///
/// `usize::MAX / 2` so the Floyd-Warshall relaxation step (`a + b`) cannot
/// overflow even if it ever combined two sentinels.
pub const UNREACHABLE: usize = usize::MAX / 2;

/// A primitive gate application: the unit a [`NativeGate`] decomposes into.
///
/// Real decomposition logic is a later phase; for issue #3 every native gate
/// uses an identity ("passthrough") decomposition.
#[derive(Debug, Clone, PartialEq)]
pub struct GateOp {
    pub name: String,
    pub qubits: Vec<usize>,
    pub params: Vec<f64>,
}

/// A gate's decomposition: maps gate parameters to the primitive sequence the
/// hardware runs. `Send + Sync` so targets can be shared across threads.
pub type DecomposeFn = Box<dyn Fn(&[f64]) -> Vec<GateOp> + Send + Sync>;

/// A gate the backend supports without decomposition (SPEC.md §8.1).
pub struct NativeGate {
    pub name: String,
    pub num_qubits: usize,
    pub decompose: DecomposeFn,
}

impl NativeGate {
    /// A gate that decomposes to itself, applied to qubits `0..num_qubits`.
    ///
    /// `#[trusted]` under Flux: the boxed-closure unsize cast is not yet
    /// supported by Flux's refinement checker, so its body is taken on faith.
    #[cfg_attr(feature = "flux", trusted)]
    pub fn passthrough(name: impl Into<String>, num_qubits: usize) -> Self {
        let name = name.into();
        let gate_name = name.clone();
        NativeGate {
            name,
            num_qubits,
            decompose: Box::new(move |params: &[f64]| {
                vec![GateOp {
                    name: gate_name.clone(),
                    qubits: (0..num_qubits).collect(),
                    params: params.to_vec(),
                }]
            }),
        }
    }
}

// `dyn Fn` is not `Debug`; print the descriptive fields and elide the closure.
impl std::fmt::Debug for NativeGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeGate")
            .field("name", &self.name)
            .field("num_qubits", &self.num_qubits)
            .field("decompose", &"<closure>")
            .finish()
    }
}

/// Hardware noise parameters (SPEC.md §8.1). Tuple keys identify the gate and
/// the qubit(s) it acts on; time/error maps are keyed by qubit index.
#[derive(Debug, Clone, Default)]
pub struct NoiseModel {
    pub single_qubit_fidelity: HashMap<(String, usize), f64>,
    pub two_qubit_fidelity: HashMap<(String, usize, usize), f64>,
    pub t1_us: HashMap<usize, f64>,
    pub t2_us: HashMap<usize, f64>,
    pub readout_error: HashMap<usize, f64>,
}

/// Physical qubit connectivity with a precomputed all-pairs shortest-path
/// matrix (Floyd-Warshall) for routing (SPEC.md §8.1).
#[derive(Debug, Clone)]
pub struct ConnectivityGraph {
    pub num_qubits: usize,
    pub edges: Vec<(usize, usize)>,
    pub dist: Vec<Vec<usize>>,
}

impl ConnectivityGraph {
    /// Fully connected topology (the `generic_openqasm` target). Valid by
    /// construction, so this is infallible.
    pub fn all_to_all(num_qubits: usize) -> Self {
        let edges: Vec<(usize, usize)> = (0..num_qubits)
            .flat_map(|i| (i + 1..num_qubits).map(move |j| (i, j)))
            .collect();
        let dist = Self::floyd_warshall(num_qubits, &edges);
        ConnectivityGraph {
            num_qubits,
            edges,
            dist,
        }
    }

    /// Build a graph from an explicit (undirected, unit-weight) edge list,
    /// validating every endpoint is in `0..num_qubits` and rejecting
    /// self-loops *before* allocating the distance matrix.
    pub fn try_from_edges(
        num_qubits: usize,
        edges: Vec<(usize, usize)>,
    ) -> Result<Self, crate::error::BackendError> {
        use crate::error::BackendError;
        for &(a, b) in &edges {
            if a == b {
                return Err(BackendError::SelfLoop(a));
            }
            if !qubit_in_range(a, num_qubits) || !qubit_in_range(b, num_qubits) {
                return Err(BackendError::EdgeOutOfRange { a, b, num_qubits });
            }
        }
        let dist = Self::floyd_warshall(num_qubits, &edges);
        Ok(ConnectivityGraph {
            num_qubits,
            edges,
            dist,
        })
    }

    /// All-pairs shortest paths over an undirected, unit-weight graph.
    /// Returns an `n × n` matrix; unreachable pairs are [`UNREACHABLE`].
    fn floyd_warshall(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
        let mut dist = vec![vec![UNREACHABLE; n]; n];
        for (d, row) in dist.iter_mut().enumerate() {
            row[d] = 0;
        }
        for &(u, v) in edges {
            // Endpoints are validated by callers; an out-of-range edge here
            // would be a construction bug, so guard rather than index blindly.
            if u < n && v < n {
                dist[u][v] = 1;
                dist[v][u] = 1;
            }
        }
        for k in 0..n {
            for i in 0..n {
                for j in 0..n {
                    let through = dist[i][k].saturating_add(dist[k][j]);
                    if through < dist[i][j] {
                        dist[i][j] = through;
                    }
                }
            }
        }
        dist
    }

    /// Shortest-path distance between two qubits. Returns [`UNREACHABLE`] if
    /// they lie in different connected components.
    pub fn dist(&self, a: usize, b: usize) -> usize {
        self.dist[a][b]
    }
}

/// A backend target descriptor with one architecture-specific payload.
///
/// Only `id` is shared at this outer level. The concrete architecture family
/// owns every other field through [`TargetKind`], following ADR-0009.
#[derive(Debug)]
pub struct BackendTarget {
    pub id: String,
    pub kind: TargetKind,
}

#[derive(Debug)]
pub enum TargetKind {
    Fixed(FixedTarget),
    NeutralAtomReconfigurable(NeutralAtomTarget),
}

/// Today's fixed-connectivity gate-model hardware descriptor: connectivity,
/// native gates, noise, and dynamic-circuit capability flags.
#[derive(Debug)]
pub struct FixedTarget {
    pub num_qubits: usize,
    pub topology: ConnectivityGraph,
    pub native_gates: Vec<NativeGate>,
    pub noise: NoiseModel,
    pub meas_latency_us: f64,
    pub supports_mid_circuit_meas: bool,
    pub supports_feed_forward: bool,
}

impl BackendTarget {
    pub fn fixed(id: impl Into<String>, fixed: FixedTarget) -> Self {
        Self {
            id: id.into(),
            kind: TargetKind::Fixed(fixed),
        }
    }

    pub fn neutral_atom_reconfigurable(
        id: impl Into<String>,
        neutral_atom: NeutralAtomTarget,
    ) -> Self {
        Self {
            id: id.into(),
            kind: TargetKind::NeutralAtomReconfigurable(neutral_atom),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            TargetKind::Fixed(_) => "fixed",
            TargetKind::NeutralAtomReconfigurable(_) => "neutral_atom_reconfigurable",
        }
    }

    pub fn fixed_target(&self) -> Option<&FixedTarget> {
        match &self.kind {
            TargetKind::Fixed(target) => Some(target),
            TargetKind::NeutralAtomReconfigurable(_) => None,
        }
    }

    pub fn neutral_atom_target(&self) -> Option<&NeutralAtomTarget> {
        match &self.kind {
            TargetKind::Fixed(_) => None,
            TargetKind::NeutralAtomReconfigurable(target) => Some(target),
        }
    }

    /// True if a gate with this name is in the native set.
    pub fn is_native(&self, gate: &str) -> bool {
        match &self.kind {
            TargetKind::Fixed(target) => target.is_native(gate),
            TargetKind::NeutralAtomReconfigurable(target) => target.is_native(gate),
        }
    }

    pub fn native_gate_names(&self) -> Vec<&str> {
        match &self.kind {
            TargetKind::Fixed(target) => target
                .native_gates
                .iter()
                .map(|g| g.name.as_str())
                .collect(),
            TargetKind::NeutralAtomReconfigurable(target) => {
                target.native_gates.iter().map(String::as_str).collect()
            }
        }
    }
}

impl FixedTarget {
    /// True if a gate with this name is in the fixed target's native set.
    pub fn is_native(&self, gate: &str) -> bool {
        self.native_gates.iter().any(|g| g.name == gate)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeutralAtomTarget {
    pub grid: NeutralAtomGrid,
    pub zones: Vec<NeutralAtomZone>,
    pub movement: AodMovement,
    pub interaction: RydbergInteraction,
    pub native_gates: Vec<String>,
    pub timing: NeutralAtomTiming,
    pub fidelity: NeutralAtomFidelity,
    /// Optional physical error probabilities for QEC reporting / experiment emit.
    /// Absent when QEC error artifacts are requested is a hard failure — never
    /// derived from [`Self::fidelity`] (ADR-0017).
    pub error_model: Option<NeutralAtomErrorModel>,
    pub cost_model: NeutralAtomCostModel,
}

impl NeutralAtomTarget {
    pub fn is_native(&self, gate: &str) -> bool {
        self.native_gates.iter().any(|g| g == gate)
    }

    /// Return the physical error model, or [`BackendError::MissingErrorModel`].
    ///
    /// Call this when QEC error-budget reporting or `--emit-qec-experiment` is
    /// requested. Do not convert from `fidelity`.
    pub fn require_error_model(
        &self,
    ) -> Result<&NeutralAtomErrorModel, crate::error::BackendError> {
        self.error_model
            .as_ref()
            .ok_or(crate::error::BackendError::MissingErrorModel)
    }

    /// Sum of `rows * cols` over zones of `kind`.
    ///
    /// `#[trusted]` under Flux: Flux ICE's on the iterator/filter/map chain
    /// (`flux-infer` projections impossible case) while checking this body.
    #[cfg_attr(feature = "flux", trusted)]
    pub fn zone_capacity(&self, kind: ZoneKind) -> u64 {
        self.zones
            .iter()
            .filter(|zone| zone.kind == kind)
            .map(|zone| u64::from(zone.rows) * u64::from(zone.cols))
            .sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NeutralAtomGrid {
    pub width_um: f64,
    pub height_um: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeutralAtomZone {
    pub zone_id: u32,
    pub kind: ZoneKind,
    pub rows: u32,
    pub cols: u32,
    pub origin_um: (f64, f64),
    pub site_pitch_um: (f64, f64),
    pub pair_gap_um: Option<f64>,
}

impl NeutralAtomZone {
    pub fn width_um(&self) -> f64 {
        f64::from(self.cols) * self.site_pitch_um.0
    }

    pub fn height_um(&self) -> f64 {
        f64::from(self.rows) * self.site_pitch_um.1
    }

    pub fn bounds_um(&self) -> (f64, f64, f64, f64) {
        let x_min = self.origin_um.0;
        let y_min = self.origin_um.1;
        (
            x_min,
            y_min,
            x_min + self.width_um(),
            y_min + self.height_um(),
        )
    }
}

/// Zone capability taxonomy for neutral-atom reconfigurable targets.
///
/// The single `ZoneKind` for the workspace (issue #212): owned here in
/// `backend` alongside [`NeutralAtomZone`], serialized directly as the JSON
/// wire form (`crate::descriptor`), and re-exported by `quon_na` for the
/// zoned placer (`quon_na::zoned`) rather than duplicated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneKind {
    Storage,
    Entanglement,
    Readout,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AodMovement {
    pub model: AodMovementModel,
    pub aod_rows: u32,
    pub aod_cols: u32,
    pub num_aods: u32,
    pub min_row_col_separation_um: f64,
    pub speed_model: AodSpeedModel,
    pub trap_transfer_us: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AodMovementModel {
    RowColumnCoupled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AodSpeedModel {
    pub kind: AodSpeedModelKind,
    pub acceleration_m_s2: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AodSpeedModelKind {
    Sqrt,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RydbergInteraction {
    pub rydberg_range_um: f64,
    pub min_rydberg_spacing_um: f64,
    pub max_parallel_entangling_pairs: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeutralAtomTiming {
    pub cz_us: f64,
    pub single_qubit_us: f64,
    pub measurement_us: f64,
    pub reset_us: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeutralAtomFidelity {
    pub cz: f64,
    pub single_qubit: f64,
    pub atom_transfer: f64,
    pub coherence_time_us: f64,
}

/// Explicit physical error probabilities for QEC (ADR-0017).
///
/// Sibling to [`NeutralAtomFidelity`]; not derived as `1 - fidelity`.
/// Wire JSON lives in [`crate::descriptor::NeutralAtomErrorModelDescriptor`]
/// with validated conversion — this domain type has no serde derives.
///
/// See also [`NeutralAtomErrorModel::error_model_snapshot`] for the stable
/// experiment-JSON DTO reused by issue #255.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NeutralAtomErrorModel {
    /// Per Rydberg illumination stage (paired with `rydberg_stages`, not per CZ).
    pub rydberg: f64,
    /// Per measurement round (`measurement_rounds`).
    pub measurement: f64,
    /// Per reset round (`reset_rounds`).
    pub reset: f64,
    /// Per rearrangement step (`rearrangement_steps`).
    pub movement: f64,
    /// Per trap transfer (`trap_transfers`).
    pub transfer: f64,
    /// Per microsecond of wait / idle (`wait_time_us`).
    pub idle_per_us: f64,
}

/// Stable serde DTO of [`NeutralAtomErrorModel`] for experiment JSON snapshots (#255).
///
/// Field names match the target wire form. Prefer this over serializing the
/// domain type directly.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeutralAtomErrorModelSnapshot {
    pub rydberg: f64,
    pub measurement: f64,
    pub reset: f64,
    pub movement: f64,
    pub transfer: f64,
    pub idle_per_us: f64,
}

impl NeutralAtomErrorModel {
    /// Snapshot physical rates for experiment JSON emit (#255).
    pub fn error_model_snapshot(&self) -> NeutralAtomErrorModelSnapshot {
        NeutralAtomErrorModelSnapshot {
            rydberg: self.rydberg,
            measurement: self.measurement,
            reset: self.reset,
            movement: self.movement,
            transfer: self.transfer,
            idle_per_us: self.idle_per_us,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeutralAtomCostModel {
    pub rydberg_stage_weight: f64,
    pub movement_time_weight: f64,
    pub trap_transfer_weight: f64,
    pub idle_time_weight: f64,
}

/// True iff `q` is a valid qubit index for a device with `n` qubits.
///
/// Refinement-typed: Flux proves the boolean result equals `q < n`, anchoring
/// the index-bounds reasoning used throughout edge/noise validation.
#[cfg_attr(feature = "flux", spec(fn(q: usize, n: usize) -> bool[q < n]))]
pub fn qubit_in_range(q: usize, n: usize) -> bool {
    q < n
}
