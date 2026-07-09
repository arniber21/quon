use serde::{Deserialize, Serialize};

use crate::schedule::{NeutralAtomAction, ScheduleLayer};

#[cfg(feature = "flux")]
use flux_rs::attrs::*;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceReport {
    pub rydberg_stages: u64,
    pub rearrangement_steps: u64,
    pub rearrangement_time_us: u64,
    pub trap_transfers: u64,
    pub transfer_time_us: u64,
    pub entangle2_count: u64,
    pub entangle_n_count: u64,
    pub measurement_rounds: u64,
    pub reset_rounds: u64,
    pub wait_time_us: u64,
    pub total_time_us: u64,
}

/// Simultaneous actions make a layer's elapsed time the maximum action duration.
#[cfg_attr(
    feature = "flux",
    spec(fn(current: u64, next: u64) -> u64{v: current <= v && next <= v && (v == current || v == next)})
)]
pub fn simultaneous_layer_time(current: u64, next: u64) -> u64 {
    if current >= next { current } else { next }
}

impl ResourceReport {
    pub fn from_layers(layers: &[ScheduleLayer]) -> Self {
        let mut report = ResourceReport::default();

        for layer in layers {
            let mut layer_has_rydberg = false;
            let mut layer_has_measurement = false;
            let mut layer_has_reset = false;
            let mut max_duration_us = 0;

            for action in &layer.actions {
                let duration_us = action.duration_us();
                max_duration_us = simultaneous_layer_time(max_duration_us, duration_us);

                match action {
                    NeutralAtomAction::Move(_) => {
                        report.rearrangement_steps += 1;
                        report.rearrangement_time_us += duration_us;
                    }
                    NeutralAtomAction::Transfer(_) => {
                        report.trap_transfers += 1;
                        report.transfer_time_us += duration_us;
                    }
                    NeutralAtomAction::Entangle2 { .. } => {
                        layer_has_rydberg = true;
                        report.entangle2_count += 1;
                    }
                    NeutralAtomAction::EntangleN { .. } => {
                        layer_has_rydberg = true;
                        report.entangle_n_count += 1;
                    }
                    NeutralAtomAction::Measure { .. } => {
                        layer_has_measurement = true;
                    }
                    NeutralAtomAction::Reset { .. } => {
                        layer_has_reset = true;
                    }
                    NeutralAtomAction::Wait { .. } => {
                        report.wait_time_us += duration_us;
                    }
                }
            }

            if layer_has_rydberg {
                report.rydberg_stages += 1;
            }
            if layer_has_measurement {
                report.measurement_rounds += 1;
            }
            if layer_has_reset {
                report.reset_rounds += 1;
            }

            report.total_time_us += max_duration_us;
        }

        report
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::layout::{AodTrapRef, AtomId, SiteId};
    use crate::schedule::{
        AtomMove, MeasurementBasis, MovementGroup, NeutralAtomAction, ScheduleLayer,
        TransferDirection, TrapTransfer,
    };

    fn atom(id: u32) -> AtomId {
        AtomId(id)
    }

    fn site(id: u32) -> SiteId {
        SiteId(id)
    }

    fn aod() -> AodTrapRef {
        AodTrapRef {
            aod_id: 0,
            row: 1,
            col: 2,
        }
    }

    #[test]
    fn resource_report_counts_grouped_movement_and_layer_time() {
        let layers = vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![
                    NeutralAtomAction::Move(MovementGroup {
                        duration_us: 10,
                        moves: vec![
                            AtomMove {
                                atom: atom(0),
                                from: site(0),
                                to: site(1),
                            },
                            AtomMove {
                                atom: atom(1),
                                from: site(2),
                                to: site(3),
                            },
                        ],
                    }),
                    NeutralAtomAction::Wait { duration_us: 4 },
                ],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![
                    NeutralAtomAction::Transfer(TrapTransfer {
                        atom: atom(0),
                        direction: TransferDirection::SlmToAod,
                        site: site(1),
                        aod: aod(),
                        duration_us: 6,
                    }),
                    NeutralAtomAction::Entangle2 {
                        atoms: [atom(0), atom(1)],
                        duration_us: 12,
                    },
                ],
            },
            ScheduleLayer {
                cycle: 2,
                actions: vec![
                    NeutralAtomAction::EntangleN {
                        atoms: vec![atom(0), atom(1), atom(2)],
                        duration_us: 8,
                    },
                    NeutralAtomAction::Measure {
                        atom: atom(0),
                        basis: MeasurementBasis::Z,
                        duration_us: 5,
                    },
                    NeutralAtomAction::Reset {
                        atom: atom(1),
                        duration_us: 7,
                    },
                ],
            },
        ];

        let report = ResourceReport::from_layers(&layers);

        assert_eq!(report.rearrangement_steps, 1);
        assert_eq!(report.rearrangement_time_us, 10);
        assert_eq!(report.trap_transfers, 1);
        assert_eq!(report.transfer_time_us, 6);
        assert_eq!(report.rydberg_stages, 2);
        assert_eq!(report.entangle2_count, 1);
        assert_eq!(report.entangle_n_count, 1);
        assert_eq!(report.measurement_rounds, 1);
        assert_eq!(report.reset_rounds, 1);
        assert_eq!(report.wait_time_us, 4);
        assert_eq!(report.total_time_us, 30);
    }

    #[test]
    fn empty_layers_have_zero_resource_usage() {
        assert_eq!(ResourceReport::from_layers(&[]), ResourceReport::default());
        assert_eq!(
            ResourceReport::from_layers(&[ScheduleLayer {
                cycle: 0,
                actions: Vec::new(),
            }]),
            ResourceReport::default()
        );
    }

    #[test]
    fn measurement_and_reset_rounds_count_layers_not_actions() {
        let report = ResourceReport::from_layers(&[ScheduleLayer {
            cycle: 0,
            actions: vec![
                NeutralAtomAction::Measure {
                    atom: atom(0),
                    basis: MeasurementBasis::X,
                    duration_us: 3,
                },
                NeutralAtomAction::Measure {
                    atom: atom(1),
                    basis: MeasurementBasis::Y,
                    duration_us: 5,
                },
                NeutralAtomAction::Reset {
                    atom: atom(2),
                    duration_us: 7,
                },
                NeutralAtomAction::Reset {
                    atom: atom(3),
                    duration_us: 2,
                },
            ],
        }]);

        assert_eq!(report.measurement_rounds, 1);
        assert_eq!(report.reset_rounds, 1);
        assert_eq!(report.total_time_us, 7);
    }

    #[test]
    fn simultaneous_layer_time_is_the_max() {
        for current in 0..16 {
            for next in 0..16 {
                let elapsed = simultaneous_layer_time(current, next);
                assert!(current <= elapsed);
                assert!(next <= elapsed);
                assert!(elapsed == current || elapsed == next);
            }
        }
    }

    #[test]
    fn serializes_resource_report_metrics_to_json() {
        let report = ResourceReport {
            rydberg_stages: 2,
            rearrangement_steps: 3,
            rearrangement_time_us: 17,
            trap_transfers: 5,
            transfer_time_us: 11,
            entangle2_count: 7,
            entangle_n_count: 1,
            measurement_rounds: 13,
            reset_rounds: 19,
            wait_time_us: 23,
            total_time_us: 29,
        };

        let value = match serde_json::to_value(report) {
            Ok(value) => value,
            Err(error) => panic!("resource report serialization failed: {error}"),
        };

        assert_eq!(
            value,
            json!({
                "rydberg_stages": 2,
                "rearrangement_steps": 3,
                "rearrangement_time_us": 17,
                "trap_transfers": 5,
                "transfer_time_us": 11,
                "entangle2_count": 7,
                "entangle_n_count": 1,
                "measurement_rounds": 13,
                "reset_rounds": 19,
                "wait_time_us": 23,
                "total_time_us": 29,
            })
        );
    }
}
