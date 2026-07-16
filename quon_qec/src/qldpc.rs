//! qLDPC-style workload IR and resource model (issue #285).
//!
//! Prototypes an abstract qLDPC-style QEC workload path focused on
//! compiler/resource modeling, not full decoding or threshold validation.
//!
//! The first slice represents a parity-check graph, generates a
//! syndrome-extraction workload structure, and estimates neutral-atom-relevant
//! resource pressure such as check weight, connectivity demand, ancilla
//! demand, measurement rounds, movement pressure, and peak atom demand.
//!
//! # Scope
//!
//! This is a **compiler/resource-model prototype**, not:
//! - A full decoder
//! - A threshold claim
//! - A hardware-specific calibration claim
//!
//! Unsupported features fail clearly with actionable diagnostics.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One parity check in a qLDPC code (stabilizer generator).
///
/// `check_atom` is the ancilla that measures this check. `data_atoms` are
/// the data qubits in the support. `basis` is the Pauli type (X/Z).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParityCheck {
    /// Check ancilla identifier (0-indexed within the code).
    pub check_id: u32,
    /// Pauli basis of this check.
    pub basis: CheckBasis,
    /// Data qubit ids in the support of this check.
    pub data_qubits: Vec<u32>,
}

/// Pauli basis for a qLDPC check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum CheckBasis {
    X,
    Z,
}

impl CheckBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Z => "z",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "x" | "X" => Some(Self::X),
            "z" | "Z" => Some(Self::Z),
            _ => None,
        }
    }
}

/// A parity-check graph for a qLDPC-style code.
///
/// Metadata: `n_data` data qubits, `n_checks` check ancillas, and the list
/// of parity checks. The graph edges connect checks to data qubits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParityCheckGraph {
    /// Number of data qubits.
    pub n_data: u32,
    /// Number of check ancillas.
    pub n_checks: u32,
    /// Code distance (best known, for resource estimation).
    pub distance: u32,
    /// Parity checks (stabilizer generators).
    pub checks: Vec<ParityCheck>,
}

/// Failures from building or validating a qLDPC workload.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum QldpcError {
    #[error("parity-check graph has {n_data} data qubits but a check references qubit {qubit}")]
    DataQubitOutOfRange { n_data: u32, qubit: u32 },
    #[error("parity-check graph has {n_checks} checks but check_id {check_id} is out of range")]
    CheckIdOutOfRange { n_checks: u32, check_id: u32 },
    #[error("parity-check graph has no checks (empty code)")]
    EmptyCode,
    #[error("check weight must be >= 1, got {weight}")]
    InvalidCheckWeight { weight: usize },
    #[error("distance must be >= 1, got {distance}")]
    InvalidDistance { distance: u32 },
}

impl ParityCheckGraph {
    /// Validate the parity-check graph.
    pub fn validate(&self) -> Result<(), QldpcError> {
        if self.checks.is_empty() {
            return Err(QldpcError::EmptyCode);
        }
        if self.distance == 0 {
            return Err(QldpcError::InvalidDistance { distance: 0 });
        }
        for check in &self.checks {
            if check.check_id >= self.n_checks {
                return Err(QldpcError::CheckIdOutOfRange {
                    n_checks: self.n_checks,
                    check_id: check.check_id,
                });
            }
            if check.data_qubits.is_empty() {
                return Err(QldpcError::InvalidCheckWeight { weight: 0 });
            }
            for &dq in &check.data_qubits {
                if dq >= self.n_data {
                    return Err(QldpcError::DataQubitOutOfRange {
                        n_data: self.n_data,
                        qubit: dq,
                    });
                }
            }
        }
        Ok(())
    }

    /// Maximum check weight (largest number of data qubits in any check).
    pub fn max_check_weight(&self) -> u32 {
        self.checks
            .iter()
            .map(|c| c.data_qubits.len() as u32)
            .max()
            .unwrap_or(0)
    }

    /// Average check weight.
    pub fn avg_check_weight(&self) -> f64 {
        if self.checks.is_empty() {
            return 0.0;
        }
        let total: u32 = self.checks.iter().map(|c| c.data_qubits.len() as u32).sum();
        total as f64 / self.checks.len() as f64
    }

    /// Total atoms (data + check ancillas).
    pub fn total_atoms(&self) -> u32 {
        self.n_data + self.n_checks
    }

    /// Total number of check-to-data edges (sum of all check weights).
    pub fn edge_count(&self) -> u32 {
        self.checks.iter().map(|c| c.data_qubits.len() as u32).sum()
    }
}

/// Resource estimates for a qLDPC-style workload on a neutral-atom target.
///
/// These are **compiler analytic estimates** — not sampled data and not
/// threshold claims. They estimate the resource pressure a qLDPC code would
/// place on a neutral-atom scheduler.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QldpcResourceEstimate {
    /// Number of data qubits.
    pub n_data: u32,
    /// Number of check ancillas.
    pub n_checks: u32,
    /// Code distance.
    pub distance: u32,
    /// Maximum check weight (connectivity demand per ancilla).
    pub max_check_weight: u32,
    /// Average check weight.
    pub avg_check_weight: f64,
    /// Total check-to-data edges (CNOT count per syndrome round).
    pub edge_count: u32,
    /// Number of syndrome-extraction measurement rounds.
    pub measurement_rounds: u32,
    /// Estimated movement pressure (Manhattan distance sum per round).
    pub movement_pressure: f64,
    /// Peak atom demand (data + check ancillas).
    pub peak_atoms: u32,
    /// Estimated cycles per syndrome round (max check weight × 2 for Z-then-X).
    pub estimated_cycles_per_round: u32,
}

impl QldpcResourceEstimate {
    /// Estimate resource pressure from a parity-check graph.
    ///
    /// `measurement_rounds` is the number of syndrome-extraction rounds to model.
    /// `grid_width` is the assumed atom grid width (for movement estimation).
    pub fn estimate(
        graph: &ParityCheckGraph,
        measurement_rounds: u32,
        grid_width: u32,
    ) -> Result<Self, QldpcError> {
        graph.validate()?;
        if measurement_rounds == 0 {
            return Err(QldpcError::InvalidDistance { distance: 0 });
        }
        let max_check_weight = graph.max_check_weight();
        let avg_check_weight = graph.avg_check_weight();
        let edge_count = graph.edge_count();
        let peak_atoms = graph.total_atoms();

        // Movement pressure: estimate as the average Manhattan distance between
        // check ancillas and their data qubits, assuming a row-major grid layout.
        // This is a rough proxy — actual movement depends on the NA scheduler.
        let movement_pressure = if grid_width > 0 && graph.n_data > 0 {
            let mut total_dist = 0u32;
            for check in &graph.checks {
                let check_row = check.check_id / grid_width;
                let check_col = check.check_id % grid_width;
                for &dq in &check.data_qubits {
                    let dq_row = dq / grid_width;
                    let dq_col = dq % grid_width;
                    let dr = (check_row as i32 - dq_row as i32).unsigned_abs();
                    let dc = (check_col as i32 - dq_col as i32).unsigned_abs();
                    total_dist += dr + dc;
                }
            }
            total_dist as f64 / edge_count as f64
        } else {
            0.0
        };

        // Estimated cycles per round: max check weight × 2 (Z-then-X split,
        // similar to surface code scheduling).
        let estimated_cycles_per_round = max_check_weight * 2;

        Ok(Self {
            n_data: graph.n_data,
            n_checks: graph.n_checks,
            distance: graph.distance,
            max_check_weight,
            avg_check_weight,
            edge_count,
            measurement_rounds,
            movement_pressure,
            peak_atoms,
            estimated_cycles_per_round,
        })
    }
}

/// Generate a syndrome-extraction workload from a small toy parity-check graph.
///
/// Each round entangles check ancillas with their data qubits (CNOTs), then
/// measures and resets the checks. The output is a list of per-round CNOT
/// schedules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyndromeExtractionRound {
    /// (check_atom, data_atom) CNOT pairs for this round.
    pub cnots: Vec<(u32, u32)>,
    /// Check atoms to measure at the end of this round.
    pub check_atoms_to_measure: Vec<u32>,
}

/// Generate syndrome-extraction rounds from a parity-check graph.
///
/// `n_rounds` is the number of rounds to generate. Each round is the same
/// (steady-state syndrome extraction).
pub fn generate_syndrome_rounds(
    graph: &ParityCheckGraph,
    n_rounds: u32,
) -> Result<Vec<SyndromeExtractionRound>, QldpcError> {
    graph.validate()?;
    if n_rounds == 0 {
        return Err(QldpcError::InvalidDistance { distance: 0 });
    }
    let mut rounds = Vec::with_capacity(n_rounds as usize);
    for _ in 0..n_rounds {
        let mut cnots = Vec::new();
        let mut check_atoms = Vec::new();
        for check in &graph.checks {
            check_atoms.push(check.check_id);
            for &dq in &check.data_qubits {
                // CNOT: data → check (Z-basis) or check → data (X-basis)
                match check.basis {
                    CheckBasis::Z => cnots.push((dq, check.check_id)),
                    CheckBasis::X => cnots.push((check.check_id, dq)),
                }
            }
        }
        rounds.push(SyndromeExtractionRound {
            cnots,
            check_atoms_to_measure: check_atoms,
        });
    }
    Ok(rounds)
}

/// A simple toy [[5,1,3]] code (5-qubit code) for testing.
pub fn toy_5qubit_graph() -> ParityCheckGraph {
    ParityCheckGraph {
        n_data: 5,
        n_checks: 4,
        distance: 3,
        checks: vec![
            ParityCheck {
                check_id: 0,
                basis: CheckBasis::Z,
                data_qubits: vec![0, 1, 2, 3, 4],
            },
            ParityCheck {
                check_id: 1,
                basis: CheckBasis::Z,
                data_qubits: vec![0, 1, 2, 3, 4],
            },
            ParityCheck {
                check_id: 2,
                basis: CheckBasis::X,
                data_qubits: vec![0, 1, 2, 3, 4],
            },
            ParityCheck {
                check_id: 3,
                basis: CheckBasis::X,
                data_qubits: vec![0, 1, 2, 3, 4],
            },
        ],
    }
}

/// A simple toy repetition code for testing.
pub fn toy_repetition_graph(distance: u32) -> ParityCheckGraph {
    let n_data = distance;
    let n_checks = distance - 1;
    let mut checks = Vec::new();
    for i in 0..n_checks {
        checks.push(ParityCheck {
            check_id: i,
            basis: CheckBasis::Z,
            data_qubits: vec![i, i + 1],
        });
    }
    ParityCheckGraph {
        n_data,
        n_checks,
        distance,
        checks,
    }
}

/// Unsupported features that fail clearly.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum QldpcUnsupportedError {
    #[error("full decoder not implemented — this is a compiler/resource model only")]
    FullDecoderNotImplemented,
    #[error("threshold claim not supported — this is a compiler/resource model only")]
    ThresholdNotSupported,
    #[error("hardware-specific calibration not supported — use generic public assumptions")]
    HardwareCalibrationNotSupported,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toy_5qubit_graph_validates() {
        let graph = toy_5qubit_graph();
        graph.validate().expect("valid");
        assert_eq!(graph.n_data, 5);
        assert_eq!(graph.n_checks, 4);
        assert_eq!(graph.max_check_weight(), 5);
    }

    #[test]
    fn toy_repetition_graph_validates() {
        let graph = toy_repetition_graph(5);
        graph.validate().expect("valid");
        assert_eq!(graph.n_data, 5);
        assert_eq!(graph.n_checks, 4);
        assert_eq!(graph.max_check_weight(), 2);
    }

    #[test]
    fn rejects_empty_code() {
        let graph = ParityCheckGraph {
            n_data: 5,
            n_checks: 0,
            distance: 3,
            checks: vec![],
        };
        assert_eq!(graph.validate(), Err(QldpcError::EmptyCode));
    }

    #[test]
    fn rejects_data_qubit_out_of_range() {
        let graph = ParityCheckGraph {
            n_data: 3,
            n_checks: 1,
            distance: 1,
            checks: vec![ParityCheck {
                check_id: 0,
                basis: CheckBasis::Z,
                data_qubits: vec![0, 3], // 3 >= n_data
            }],
        };
        assert_eq!(
            graph.validate(),
            Err(QldpcError::DataQubitOutOfRange {
                n_data: 3,
                qubit: 3
            })
        );
    }

    #[test]
    fn rejects_check_id_out_of_range() {
        let graph = ParityCheckGraph {
            n_data: 3,
            n_checks: 1,
            distance: 1,
            checks: vec![ParityCheck {
                check_id: 5, // >= n_checks
                basis: CheckBasis::Z,
                data_qubits: vec![0, 1],
            }],
        };
        assert_eq!(
            graph.validate(),
            Err(QldpcError::CheckIdOutOfRange {
                n_checks: 1,
                check_id: 5
            })
        );
    }

    #[test]
    fn resource_estimate_5qubit() {
        let graph = toy_5qubit_graph();
        let est = QldpcResourceEstimate::estimate(&graph, 3, 5).expect("estimate");
        assert_eq!(est.n_data, 5);
        assert_eq!(est.n_checks, 4);
        assert_eq!(est.distance, 3);
        assert_eq!(est.max_check_weight, 5);
        assert_eq!(est.edge_count, 20); // 4 checks × 5 data
        assert_eq!(est.measurement_rounds, 3);
        assert_eq!(est.peak_atoms, 9); // 5 data + 4 checks
        assert_eq!(est.estimated_cycles_per_round, 10); // 5 × 2
    }

    #[test]
    fn resource_estimate_repetition() {
        let graph = toy_repetition_graph(5);
        let est = QldpcResourceEstimate::estimate(&graph, 2, 5).expect("estimate");
        assert_eq!(est.max_check_weight, 2);
        assert_eq!(est.edge_count, 8); // 4 checks × 2 data
        assert_eq!(est.peak_atoms, 9); // 5 data + 4 checks
        assert_eq!(est.estimated_cycles_per_round, 4); // 2 × 2
    }

    #[test]
    fn movement_pressure_is_nonnegative() {
        let graph = toy_5qubit_graph();
        let est = QldpcResourceEstimate::estimate(&graph, 1, 5).expect("estimate");
        assert!(est.movement_pressure >= 0.0);
    }

    #[test]
    fn movement_pressure_zero_with_zero_grid() {
        let graph = toy_5qubit_graph();
        let est = QldpcResourceEstimate::estimate(&graph, 1, 0).expect("estimate");
        assert_eq!(est.movement_pressure, 0.0);
    }

    #[test]
    fn syndrome_rounds_generate_correct_cnots() {
        let graph = toy_repetition_graph(5);
        let rounds = generate_syndrome_rounds(&graph, 2).expect("rounds");
        assert_eq!(rounds.len(), 2);
        // Each round has 4 checks × 2 data = 8 CNOTs
        assert_eq!(rounds[0].cnots.len(), 8);
        // Check atoms to measure = 4
        assert_eq!(rounds[0].check_atoms_to_measure.len(), 4);
        // Both rounds are identical (steady state)
        assert_eq!(rounds[0], rounds[1]);
    }

    #[test]
    fn syndrome_rounds_x_basis_uses_check_as_control() {
        let graph = ParityCheckGraph {
            n_data: 3,
            n_checks: 1,
            distance: 1,
            checks: vec![ParityCheck {
                check_id: 0,
                basis: CheckBasis::X,
                data_qubits: vec![0, 1, 2],
            }],
        };
        let rounds = generate_syndrome_rounds(&graph, 1).expect("rounds");
        // X-basis: check is control, data is target
        assert_eq!(rounds[0].cnots[0], (0, 0)); // (check_id=0, data=0)
        assert_eq!(rounds[0].cnots[1], (0, 1)); // (check_id=0, data=1)
    }

    #[test]
    fn syndrome_rounds_z_basis_uses_data_as_control() {
        let graph = ParityCheckGraph {
            n_data: 3,
            n_checks: 1,
            distance: 1,
            checks: vec![ParityCheck {
                check_id: 0,
                basis: CheckBasis::Z,
                data_qubits: vec![0, 1, 2],
            }],
        };
        let rounds = generate_syndrome_rounds(&graph, 1).expect("rounds");
        // Z-basis: data is control, check is target
        assert_eq!(rounds[0].cnots[0], (0, 0)); // (data=0, check_id=0)
    }

    #[test]
    fn unsupported_features_fail_clearly() {
        assert_eq!(
            QldpcUnsupportedError::FullDecoderNotImplemented.to_string(),
            "full decoder not implemented — this is a compiler/resource model only"
        );
        assert_eq!(
            QldpcUnsupportedError::ThresholdNotSupported.to_string(),
            "threshold claim not supported — this is a compiler/resource model only"
        );
        assert_eq!(
            QldpcUnsupportedError::HardwareCalibrationNotSupported.to_string(),
            "hardware-specific calibration not supported — use generic public assumptions"
        );
    }

    #[test]
    fn avg_check_weight_correct() {
        let graph = ParityCheckGraph {
            n_data: 6,
            n_checks: 2,
            distance: 1,
            checks: vec![
                ParityCheck {
                    check_id: 0,
                    basis: CheckBasis::Z,
                    data_qubits: vec![0, 1, 2],
                },
                ParityCheck {
                    check_id: 1,
                    basis: CheckBasis::Z,
                    data_qubits: vec![3, 4],
                },
            ],
        };
        assert!((graph.avg_check_weight() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn rejects_zero_distance() {
        let graph = ParityCheckGraph {
            n_data: 3,
            n_checks: 1,
            distance: 0,
            checks: vec![ParityCheck {
                check_id: 0,
                basis: CheckBasis::Z,
                data_qubits: vec![0, 1],
            }],
        };
        assert_eq!(
            graph.validate(),
            Err(QldpcError::InvalidDistance { distance: 0 })
        );
    }

    #[test]
    fn rejects_zero_rounds() {
        let graph = toy_5qubit_graph();
        assert!(generate_syndrome_rounds(&graph, 0).is_err());
    }
}
