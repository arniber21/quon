use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::layout::{AodTrapRef, AtomId, SiteId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum NeutralAtomAction {
    Move(MovementGroup),
    Transfer(TrapTransfer),
    Entangle2 {
        atoms: [AtomId; 2],
        duration_us: u64,
    },
    EntangleN {
        atoms: Vec<AtomId>,
        duration_us: u64,
    },
    Measure {
        atom: AtomId,
        basis: MeasurementBasis,
        duration_us: u64,
    },
    Reset {
        atom: AtomId,
        duration_us: u64,
    },
    Wait {
        duration_us: u64,
    },
}

impl NeutralAtomAction {
    pub fn duration_us(&self) -> u64 {
        match self {
            NeutralAtomAction::Move(group) => group.duration_us,
            NeutralAtomAction::Transfer(transfer) => transfer.duration_us,
            NeutralAtomAction::Entangle2 { duration_us, .. }
            | NeutralAtomAction::EntangleN { duration_us, .. }
            | NeutralAtomAction::Measure { duration_us, .. }
            | NeutralAtomAction::Reset { duration_us, .. }
            | NeutralAtomAction::Wait { duration_us } => *duration_us,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementGroup {
    pub moves: Vec<AtomMove>,
    pub duration_us: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtomMove {
    pub atom: AtomId,
    pub from: SiteId,
    pub to: SiteId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrapTransfer {
    pub atom: AtomId,
    pub direction: TransferDirection,
    pub site: SiteId,
    pub aod: AodTrapRef,
    pub duration_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDirection {
    SlmToAod,
    AodToSlm,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntanglingAction {
    pub atoms: Vec<AtomId>,
    pub duration_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeasurementBasis {
    X,
    Y,
    Z,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleLayer {
    pub cycle: u32,
    pub actions: Vec<NeutralAtomAction>,
}

impl ScheduleLayer {
    pub fn validate_conflicts(&self) -> Result<(), ScheduleError> {
        let mut entangling_atoms = BTreeSet::new();

        for action in &self.actions {
            match action {
                NeutralAtomAction::Entangle2 { atoms, .. } => {
                    for atom in atoms {
                        if !entangling_atoms.insert(*atom) {
                            return Err(ScheduleError::DuplicateEntanglingAtom {
                                cycle: self.cycle,
                                atom: *atom,
                            });
                        }
                    }
                }
                NeutralAtomAction::EntangleN { atoms, .. } => {
                    for atom in atoms {
                        if !entangling_atoms.insert(*atom) {
                            return Err(ScheduleError::DuplicateEntanglingAtom {
                                cycle: self.cycle,
                                atom: *atom,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    pub fn validate_occupancy(&self) -> Result<(), ScheduleError> {
        let mut claimed_atoms = BTreeSet::new();
        let mut claimed_sites = BTreeSet::new();

        for action in &self.actions {
            match action {
                NeutralAtomAction::Move(group) => {
                    for atom_move in &group.moves {
                        self.claim_occupancy(
                            &mut claimed_atoms,
                            &mut claimed_sites,
                            atom_move.atom,
                            atom_move.to,
                        )?;
                    }
                }
                NeutralAtomAction::Transfer(transfer) => {
                    self.claim_occupancy(
                        &mut claimed_atoms,
                        &mut claimed_sites,
                        transfer.atom,
                        transfer.site,
                    )?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn claim_occupancy(
        &self,
        claimed_atoms: &mut BTreeSet<AtomId>,
        claimed_sites: &mut BTreeSet<SiteId>,
        atom: AtomId,
        site: SiteId,
    ) -> Result<(), ScheduleError> {
        if !claimed_atoms.insert(atom) {
            return Err(ScheduleError::DuplicateOccupancyAtom {
                cycle: self.cycle,
                atom,
            });
        }

        if !claimed_sites.insert(site) {
            return Err(ScheduleError::DuplicateOccupancySite {
                cycle: self.cycle,
                site,
            });
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleError {
    DuplicateEntanglingAtom { cycle: u32, atom: AtomId },
    DuplicateOccupancyAtom { cycle: u32, atom: AtomId },
    DuplicateOccupancySite { cycle: u32, site: SiteId },
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScheduleError::DuplicateEntanglingAtom { cycle, atom } => {
                write!(
                    f,
                    "atom {:?} participates in multiple entangling actions in cycle {}",
                    atom, cycle
                )
            }
            ScheduleError::DuplicateOccupancyAtom { cycle, atom } => {
                write!(
                    f,
                    "atom {:?} has multiple movement or transfer claims in cycle {}",
                    atom, cycle
                )
            }
            ScheduleError::DuplicateOccupancySite { cycle, site } => {
                write!(
                    f,
                    "site {:?} has multiple movement or transfer destination claims in cycle {}",
                    site, cycle
                )
            }
        }
    }
}

impl Error for ScheduleError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::AodTrapRef;

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
    fn accepts_distinct_movement_destinations() {
        let layer = ScheduleLayer {
            cycle: 3,
            actions: vec![NeutralAtomAction::Move(MovementGroup {
                duration_us: 10,
                moves: vec![
                    AtomMove {
                        atom: atom(0),
                        from: site(0),
                        to: site(10),
                    },
                    AtomMove {
                        atom: atom(1),
                        from: site(1),
                        to: site(11),
                    },
                ],
            })],
        };

        assert_eq!(layer.validate_occupancy(), Ok(()));
    }

    #[test]
    fn rejects_duplicate_same_cycle_site_occupancy() {
        let layer = ScheduleLayer {
            cycle: 4,
            actions: vec![NeutralAtomAction::Move(MovementGroup {
                duration_us: 10,
                moves: vec![
                    AtomMove {
                        atom: atom(0),
                        from: site(0),
                        to: site(10),
                    },
                    AtomMove {
                        atom: atom(1),
                        from: site(1),
                        to: site(10),
                    },
                ],
            })],
        };

        assert_eq!(
            layer.validate_occupancy(),
            Err(ScheduleError::DuplicateOccupancySite {
                cycle: 4,
                site: site(10),
            })
        );
    }

    #[test]
    fn rejects_duplicate_same_cycle_atom_occupancy_claims() {
        let layer = ScheduleLayer {
            cycle: 4,
            actions: vec![
                NeutralAtomAction::Move(MovementGroup {
                    duration_us: 10,
                    moves: vec![AtomMove {
                        atom: atom(0),
                        from: site(0),
                        to: site(10),
                    }],
                }),
                NeutralAtomAction::Transfer(TrapTransfer {
                    atom: atom(0),
                    direction: TransferDirection::SlmToAod,
                    site: site(12),
                    aod: aod(),
                    duration_us: 5,
                }),
            ],
        };

        assert_eq!(
            layer.validate_occupancy(),
            Err(ScheduleError::DuplicateOccupancyAtom {
                cycle: 4,
                atom: atom(0),
            })
        );
    }

    #[test]
    fn rejects_transfer_and_move_to_same_site() {
        let layer = ScheduleLayer {
            cycle: 7,
            actions: vec![
                NeutralAtomAction::Move(MovementGroup {
                    duration_us: 10,
                    moves: vec![AtomMove {
                        atom: atom(0),
                        from: site(0),
                        to: site(10),
                    }],
                }),
                NeutralAtomAction::Transfer(TrapTransfer {
                    atom: atom(1),
                    direction: TransferDirection::AodToSlm,
                    site: site(10),
                    aod: aod(),
                    duration_us: 5,
                }),
            ],
        };

        assert_eq!(
            layer.validate_occupancy(),
            Err(ScheduleError::DuplicateOccupancySite {
                cycle: 7,
                site: site(10),
            })
        );
    }

    #[test]
    fn rejects_atom_used_in_two_same_cycle_entangling_actions() {
        let layer = ScheduleLayer {
            cycle: 9,
            actions: vec![
                NeutralAtomAction::Entangle2 {
                    atoms: [atom(0), atom(1)],
                    duration_us: 3,
                },
                NeutralAtomAction::EntangleN {
                    atoms: vec![atom(1), atom(2), atom(3)],
                    duration_us: 4,
                },
            ],
        };

        assert_eq!(
            layer.validate_conflicts(),
            Err(ScheduleError::DuplicateEntanglingAtom {
                cycle: 9,
                atom: atom(1),
            })
        );
    }

    #[test]
    fn rejects_duplicate_atom_inside_one_entangle_n() {
        let layer = ScheduleLayer {
            cycle: 10,
            actions: vec![NeutralAtomAction::EntangleN {
                atoms: vec![atom(0), atom(1), atom(0)],
                duration_us: 4,
            }],
        };

        assert_eq!(
            layer.validate_conflicts(),
            Err(ScheduleError::DuplicateEntanglingAtom {
                cycle: 10,
                atom: atom(0),
            })
        );
    }
}
