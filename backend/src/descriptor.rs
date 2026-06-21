// Target descriptor DTO and conversion — see issue #3, SPEC.md §8.3.
//
// `TargetDescriptor` mirrors the §8.3 JSON wire format exactly (string-keyed
// noise objects, gate names as strings). `TryFrom<TargetDescriptor>` converts
// it into the SPEC §8.1 domain `BackendTarget`, resolving gate names, building
// the connectivity graph (Floyd-Warshall), and flattening the noise maps into
// tuple-keyed form. `deny_unknown_fields` rejects typos in user JSON.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::BackendError;
use crate::gates;
use crate::keys;
use crate::target::{BackendTarget, ConnectivityGraph, NoiseModel};

/// The §8.3 JSON shape. Fields without `#[serde(default)]` are required, so a
/// missing `num_qubits` or `native_gates` is rejected by serde with an error
/// that names the field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetDescriptor {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyDescriptor {
    pub edges: Vec<(usize, usize)>,
}

/// Noise as it appears in JSON: gate → qubit-string → value (string-keyed
/// because JSON object keys are strings). Every field defaults to empty.
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

impl TryFrom<TargetDescriptor> for BackendTarget {
    type Error = BackendError;

    fn try_from(d: TargetDescriptor) -> Result<Self, Self::Error> {
        let topology = ConnectivityGraph::try_from_edges(d.num_qubits, d.topology.edges)?;
        let native_gates = d
            .native_gates
            .iter()
            .map(|name| gates::native_gate(name))
            .collect::<Result<Vec<_>, _>>()?;
        let noise = d.noise.into_model(d.num_qubits)?;
        Ok(BackendTarget {
            id: d.id,
            num_qubits: d.num_qubits,
            topology,
            native_gates,
            noise,
            meas_latency_us: d.meas_latency_us,
            supports_mid_circuit_meas: d.supports_mid_circuit_meas,
            supports_feed_forward: d.supports_feed_forward,
        })
    }
}

impl BackendTarget {
    /// Project back to the §8.3 wire form (drops decomposition closures). Used
    /// for serialization and round-trip testing.
    pub fn to_descriptor(&self) -> TargetDescriptor {
        let mut noise = NoiseDescriptor::default();
        for ((gate, q), fid) in &self.noise.single_qubit_fidelity {
            noise
                .single_qubit_fidelity
                .entry(gate.clone())
                .or_default()
                .insert(keys::encode_qubit(*q), *fid);
        }
        for ((gate, u, v), fid) in &self.noise.two_qubit_fidelity {
            noise
                .two_qubit_fidelity
                .entry(gate.clone())
                .or_default()
                .insert(keys::encode_pair(*u, *v), *fid);
        }
        noise.t1_us = self
            .noise
            .t1_us
            .iter()
            .map(|(q, t)| (keys::encode_qubit(*q), *t))
            .collect();
        noise.t2_us = self
            .noise
            .t2_us
            .iter()
            .map(|(q, t)| (keys::encode_qubit(*q), *t))
            .collect();
        noise.readout_error = self
            .noise
            .readout_error
            .iter()
            .map(|(q, e)| (keys::encode_qubit(*q), *e))
            .collect();

        TargetDescriptor {
            id: self.id.clone(),
            num_qubits: self.num_qubits,
            topology: TopologyDescriptor {
                edges: self.topology.edges.clone(),
            },
            native_gates: self.native_gates.iter().map(|g| g.name.clone()).collect(),
            noise,
            meas_latency_us: self.meas_latency_us,
            supports_mid_circuit_meas: self.supports_mid_circuit_meas,
            supports_feed_forward: self.supports_feed_forward,
        }
    }
}
