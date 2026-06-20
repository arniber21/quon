// BackendTarget and supporting types — see issue #3, SPEC.md §8.1.
//
// The in-memory domain types here mirror SPEC.md §8.1 exactly, including the
// non-serializable `decompose` closure on `NativeGate`. Serialization lives in
// `crate::descriptor` (the §8.3 JSON wire format) with a `TryFrom` conversion.

#[cfg(feature = "flux")]
use flux_rs::attrs::*;
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

/// A hardware descriptor: connectivity, native gates, noise, and capability
/// flags (SPEC.md §8.1, glossary `BackendTarget`).
#[derive(Debug)]
pub struct BackendTarget {
    pub id: String,
    pub num_qubits: usize,
    pub topology: ConnectivityGraph,
    pub native_gates: Vec<NativeGate>,
    pub noise: NoiseModel,
    pub meas_latency_us: f64,
    pub supports_mid_circuit_meas: bool,
    pub supports_feed_forward: bool,
}

impl BackendTarget {
    /// True if a gate with this name is in the native set.
    pub fn is_native(&self, gate: &str) -> bool {
        self.native_gates.iter().any(|g| g.name == gate)
    }
}

/// True iff `q` is a valid qubit index for a device with `n` qubits.
///
/// Refinement-typed: Flux proves the boolean result equals `q < n`, anchoring
/// the index-bounds reasoning used throughout edge/noise validation.
#[cfg_attr(feature = "flux", spec(fn(q: usize, n: usize) -> bool[q < n]))]
pub fn qubit_in_range(q: usize, n: usize) -> bool {
    q < n
}
