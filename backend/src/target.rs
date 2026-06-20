// BackendTarget and supporting types — see issue #3, SPEC.md §8.1

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendTarget {
    pub id: String,
    pub num_qubits: usize,
    pub topology: ConnectivityGraph,
    pub native_gates: Vec<String>,
    pub noise: NoiseModel,
    pub meas_latency_us: f64,
    pub supports_mid_circuit_meas: bool,
    pub supports_feed_forward: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivityGraph {
    pub num_qubits: usize,
    pub edges: Vec<(usize, usize)>,
    #[serde(skip)]
    pub dist: Vec<Vec<usize>>, // Floyd-Warshall, precomputed at construction
}

impl ConnectivityGraph {
    pub fn new(num_qubits: usize, edges: Vec<(usize, usize)>) -> Self {
        let dist = Self::floyd_warshall(num_qubits, &edges);
        Self {
            num_qubits,
            edges,
            dist,
        }
    }

    fn floyd_warshall(_n: usize, _edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
        todo!("Floyd-Warshall — see issue #3")
    }

    pub fn dist(&self, a: usize, b: usize) -> usize {
        self.dist[a][b]
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoiseModel {
    pub single_qubit_fidelity: HashMap<String, HashMap<usize, f64>>,
    pub two_qubit_fidelity: HashMap<String, HashMap<String, f64>>, // "u,v" key
    pub t1_us: HashMap<usize, f64>,
    pub t2_us: HashMap<usize, f64>,
    pub readout_error: HashMap<usize, f64>,
}
