use melior::Context;
use melior::ir::attribute::{FloatAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationBuilder;
use melior::ir::r#type::IntegerType;
use melior::ir::{
    Attribute, Block, BlockLike, Identifier, Location, Module, Operation, Region, RegionLike, Type,
};

use quon_na::dialect::attr;
use quon_na::dialect::{
    self as qna, ActionSpec, EntanglePairSpec, LayerSpec, MoveSpec, PositionedAtom, ScheduleSpec,
    TransferDirection, TransferSpec, VerifyError,
};

fn context() -> Context {
    let context = Context::new();
    qna::register_dialect(&context);
    context
}

fn base_move(atom: u32, from_site: u32, to_site: u32, row: u32, col: u32) -> MoveSpec {
    MoveSpec {
        atom,
        from_site,
        to_site,
        aod_id: 0,
        row,
        col,
        from_x_um: f64::from(col) * 10.0,
        from_y_um: f64::from(row) * 10.0,
        to_x_um: f64::from(col) * 10.0,
        to_y_um: f64::from(row) * 10.0 + 2.0,
    }
}

fn atom(atom: u32, x_um: f64, y_um: f64) -> PositionedAtom {
    PositionedAtom { atom, x_um, y_um }
}

fn pair(lhs: PositionedAtom, rhs: PositionedAtom) -> EntanglePairSpec {
    EntanglePairSpec { lhs, rhs }
}

fn valid_spec() -> ScheduleSpec {
    ScheduleSpec {
        target_id: "generic_reconfigurable_neutral_atom_v0".to_string(),
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
        layers: vec![
            LayerSpec {
                cycle: 0,
                actions: vec![ActionSpec::Move {
                    moves: vec![base_move(0, 0, 10, 0, 0), base_move(1, 1, 11, 1, 1)],
                    duration_us: 20,
                }],
            },
            LayerSpec {
                cycle: 1,
                actions: vec![ActionSpec::Entangle {
                    pairs: vec![
                        pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0)),
                        pair(atom(2, 30.0, 0.0), atom(3, 36.0, 0.0)),
                    ],
                    duration_us: 1,
                }],
            },
            LayerSpec {
                cycle: 2,
                actions: vec![
                    ActionSpec::Measure {
                        atom: 0,
                        basis: "z".to_string(),
                        duration_us: 1500,
                    },
                    ActionSpec::Reset {
                        atom: 1,
                        duration_us: 1500,
                    },
                    ActionSpec::Wait { duration_us: 3 },
                ],
            },
        ],
    }
}

fn verify_spec(spec: &ScheduleSpec) -> Result<(), VerifyError> {
    let context = context();
    let module = match qna::schedule_module(&context, spec) {
        Ok(module) => module,
        Err(qna::BuildError::Verify(error)) => return Err(error),
        Err(error) => panic!("schedule module build failed unexpectedly: {error}"),
    };
    let schedule = module.body().first_operation().expect("schedule op exists");
    qna::verify(&schedule)
}

#[test]
fn registration_is_idempotent_and_panic_free() {
    let context = context();
    qna::register_dialect(&context);
    assert!(context.allow_unregistered_dialects());
    assert_eq!(qna::OPS.len(), 12);
}

#[test]
fn type_helpers_print_canonically() {
    let context = context();
    assert_eq!(qna::atom_type(&context).to_string(), qna::ATOM_TYPE);
    assert_eq!(qna::site_type(&context).to_string(), qna::SITE_TYPE);
    assert_eq!(qna::bit_type(&context).to_string(), qna::BIT_TYPE);
}

#[test]
fn textual_dump_round_trips_in_generic_form() {
    let spec = valid_spec();
    let text = qna::dump_schedule_text(&spec).expect("dump schedule");

    assert!(text.contains("\"quantum.na.schedule\""));
    assert!(text.contains("\"quantum.na.layer\""));
    assert!(text.contains("\"quantum.na.move\""));
    assert!(text.contains("\"quantum.na.entangle\""));

    let context = context();
    let reparsed = Module::parse(&context, &text).expect("round-trip parse");
    assert_eq!(text, reparsed.as_operation().to_string());
}

#[test]
fn verifier_accepts_valid_schedule() {
    assert_eq!(verify_spec(&valid_spec()), Ok(()));
}

#[test]
fn verifier_rejects_duplicate_atom_occupancy_in_one_cycle() {
    let mut spec = valid_spec();
    spec.layers[0].actions = vec![ActionSpec::Move {
        moves: vec![base_move(0, 0, 10, 0, 0), base_move(0, 1, 11, 1, 1)],
        duration_us: 20,
    }];

    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::DuplicateOccupancyAtom { cycle: 0, atom: 0 })
    );
}

#[test]
fn verifier_rejects_duplicate_site_occupancy_in_one_cycle() {
    let mut spec = valid_spec();
    spec.layers[0].actions = vec![ActionSpec::Move {
        moves: vec![base_move(0, 0, 10, 0, 0), base_move(1, 1, 10, 1, 1)],
        duration_us: 20,
    }];

    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::DuplicateOccupancySite { cycle: 0, site: 10 })
    );
}

#[test]
fn verifier_rejects_same_atom_in_two_entangling_gates_in_one_layer() {
    let mut spec = valid_spec();
    spec.layers[1].actions = vec![ActionSpec::Entangle {
        pairs: vec![
            pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0)),
            pair(atom(0, 30.0, 0.0), atom(2, 36.0, 0.0)),
        ],
        duration_us: 1,
    }];

    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::DuplicateEntanglingAtom { cycle: 1, atom: 0 })
    );
}

#[test]
fn verifier_rejects_entangling_pair_outside_rydberg_range() {
    let mut spec = valid_spec();
    spec.layers[1].actions = vec![ActionSpec::Entangle {
        pairs: vec![pair(atom(0, 0.0, 0.0), atom(1, 8.0, 0.0))],
        duration_us: 1,
    }];

    assert!(matches!(
        verify_spec(&spec),
        Err(VerifyError::EntanglingPairOutOfRange {
            cycle: 1,
            lhs: 0,
            rhs: 1,
            ..
        })
    ));
}

#[test]
fn verifier_rejects_non_partner_inside_compulsory_range() {
    let mut spec = valid_spec();
    spec.layers[1].actions = vec![ActionSpec::Entangle {
        pairs: vec![
            pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0)),
            pair(atom(2, 4.0, 0.0), atom(3, 4.0, 6.0)),
        ],
        duration_us: 1,
    }];

    assert!(matches!(
        verify_spec(&spec),
        Err(VerifyError::CompulsoryEntanglement { cycle: 1, .. })
    ));
}

#[test]
fn verifier_rejects_non_partner_inside_isolation_spacing() {
    let mut spec = valid_spec();
    spec.layers[1].actions = vec![ActionSpec::Entangle {
        pairs: vec![
            pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0)),
            pair(atom(2, 20.0, 0.0), atom(3, 26.0, 0.0)),
        ],
        duration_us: 1,
    }];

    assert!(matches!(
        verify_spec(&spec),
        Err(VerifyError::RydbergSpacing {
            cycle: 1,
            lhs: 1,
            rhs: 2,
            ..
        })
    ));
}

#[test]
fn verifier_rejects_aod_row_coupling_violation() {
    let mut bad = base_move(1, 1, 11, 0, 1);
    bad.to_y_um += 4.0;
    let mut spec = valid_spec();
    spec.layers[0].actions = vec![ActionSpec::Move {
        moves: vec![base_move(0, 0, 10, 0, 0), bad],
        duration_us: 20,
    }];

    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::AodRowCoupling {
            cycle: 0,
            aod_id: 0,
            row: 0,
        })
    );
}

#[test]
fn verifier_rejects_aod_column_coupling_violation() {
    let mut bad = base_move(1, 1, 11, 1, 0);
    bad.to_x_um += 4.0;
    let mut spec = valid_spec();
    spec.layers[0].actions = vec![ActionSpec::Move {
        moves: vec![base_move(0, 0, 10, 0, 0), bad],
        duration_us: 20,
    }];

    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::AodColumnCoupling {
            cycle: 0,
            aod_id: 0,
            col: 0,
        })
    );
}

#[test]
fn verifier_rejects_aod_row_order_crossing() {
    let mut first = base_move(0, 0, 10, 0, 0);
    let mut second = base_move(1, 1, 11, 1, 1);
    first.to_y_um = 14.0;
    second.to_y_um = 4.0;
    let mut spec = valid_spec();
    spec.layers[0].actions = vec![ActionSpec::Move {
        moves: vec![first, second],
        duration_us: 20,
    }];

    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::AodRowOrder {
            cycle: 0,
            aod_id: 0,
            first: 0,
            second: 1,
        })
    );
}

#[test]
fn verifier_rejects_aod_column_merging() {
    let mut first = base_move(0, 0, 10, 0, 0);
    let mut second = base_move(1, 1, 11, 1, 1);
    first.to_x_um = 8.5;
    second.to_x_um = 10.0;
    let mut spec = valid_spec();
    spec.layers[0].actions = vec![ActionSpec::Move {
        moves: vec![first, second],
        duration_us: 20,
    }];

    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::AodColumnSeparation {
            cycle: 0,
            aod_id: 0,
            first: 0,
            second: 1,
            min_separation_um: 2.0,
        })
    );
}

#[test]
fn verifier_rejects_layer_with_foreign_op() {
    let context = context();
    let location = Location::unknown(&context);
    let region = Region::new();
    let block = Block::new(&[]);
    block.append_operation(generic_op(
        &context,
        "test.foreign",
        &[],
        &[],
        &[],
        location,
    ));
    region.append_block(block);

    let layer = qna::layer(&context, 0, region, location).expect_err("foreign op rejected");
    assert!(matches!(
        layer,
        qna::BuildError::Verify(VerifyError::ForbiddenOp {
            op: qna::op::LAYER,
            ..
        })
    ));
}

fn i64_attr(context: &Context, value: i64) -> Attribute<'_> {
    IntegerAttribute::new(IntegerType::new(context, 64).into(), value).into()
}

fn f64_attr(context: &Context, value: f64) -> Attribute<'_> {
    let float_type = Type::parse(context, "f64").unwrap_or_else(|| Type::none(context));
    FloatAttribute::new(context, float_type, value).into()
}

fn str_attr<'c>(context: &'c Context, value: &str) -> Attribute<'c> {
    StringAttribute::new(context, value).into()
}

fn generic_op<'c>(
    context: &'c Context,
    name: &str,
    operands: &[melior::ir::Value<'c, '_>],
    results: &[Type<'c>],
    attributes: &[(&str, Attribute<'c>)],
    location: Location<'c>,
) -> Operation<'c> {
    let attributes: Vec<(Identifier, Attribute)> = attributes
        .iter()
        .map(|(name, value)| (Identifier::new(context, name), *value))
        .collect();
    OperationBuilder::new(name, location)
        .add_operands(operands)
        .add_results(results)
        .add_attributes(&attributes)
        .build()
        .expect("generic op builds")
}

fn transfer(
    atom: u32,
    site: u32,
    row: u32,
    col: u32,
    direction: TransferDirection,
) -> TransferSpec {
    TransferSpec {
        atom,
        site,
        aod_id: 0,
        row,
        col,
        direction,
        duration_us: 15,
    }
}

#[test]
fn verifier_rejects_move_ref_inconsistent_with_load() {
    // Atom 0 is loaded into trap (0, 0, 0) but the move claims (0, 0, 5).
    let mut spec = valid_spec();
    spec.layers.insert(
        0,
        LayerSpec {
            cycle: 0,
            actions: vec![ActionSpec::Transfer(transfer(
                0,
                0,
                0,
                0,
                TransferDirection::SlmToAod,
            ))],
        },
    );
    let LayerSpec { actions, .. } = &mut spec.layers[1];
    let ActionSpec::Move { moves, .. } = &mut actions[0] else {
        panic!("expected move layer");
    };
    moves[0].col = 5;
    moves[0].from_x_um = 50.0;
    moves[0].to_x_um = 50.0;
    assert!(matches!(
        verify_spec(&spec),
        Err(VerifyError::AodRefMismatch {
            atom: 0,
            col: 5,
            bound_col: 0,
            ..
        })
    ));
}

#[test]
fn verifier_accepts_move_ref_matching_load_and_store() {
    let mut spec = valid_spec();
    spec.layers.insert(
        0,
        LayerSpec {
            cycle: 0,
            actions: vec![
                ActionSpec::Transfer(transfer(0, 0, 0, 0, TransferDirection::SlmToAod)),
                ActionSpec::Transfer(transfer(1, 1, 1, 1, TransferDirection::SlmToAod)),
            ],
        },
    );
    spec.layers.insert(
        2,
        LayerSpec {
            cycle: 2,
            actions: vec![
                ActionSpec::Transfer(transfer(0, 10, 0, 0, TransferDirection::AodToSlm)),
                ActionSpec::Transfer(transfer(1, 11, 1, 1, TransferDirection::AodToSlm)),
            ],
        },
    );
    for (index, layer) in spec.layers.iter_mut().enumerate() {
        layer.cycle = index as u32;
    }
    verify_spec(&spec).expect("consistent transfer/move refs verify");
}

#[test]
fn verifier_rejects_store_ref_inconsistent_with_load() {
    let mut spec = valid_spec();
    spec.layers.insert(
        0,
        LayerSpec {
            cycle: 0,
            actions: vec![ActionSpec::Transfer(transfer(
                0,
                0,
                0,
                0,
                TransferDirection::SlmToAod,
            ))],
        },
    );
    // Store claims a different trap than the load.
    spec.layers.insert(
        2,
        LayerSpec {
            cycle: 2,
            actions: vec![ActionSpec::Transfer(transfer(
                0,
                10,
                3,
                3,
                TransferDirection::AodToSlm,
            ))],
        },
    );
    for (index, layer) in spec.layers.iter_mut().enumerate() {
        layer.cycle = index as u32;
    }
    assert!(matches!(
        verify_spec(&spec),
        Err(VerifyError::AodRefMismatch {
            atom: 0,
            row: 3,
            bound_row: 0,
            ..
        })
    ));
}

#[test]
fn verifier_rejects_two_moves_claiming_one_trap_from_different_sources() {
    let mut spec = valid_spec();
    let LayerSpec { actions, .. } = &mut spec.layers[0];
    let ActionSpec::Move { moves, .. } = &mut actions[0] else {
        panic!("expected move layer");
    };
    // Second move claims the same (aod, row, col) as the first but starts
    // somewhere else.
    moves[1].row = moves[0].row;
    moves[1].col = moves[0].col;
    assert!(matches!(
        verify_spec(&spec),
        Err(VerifyError::AodTrapDoubleClaim {
            aod_id: 0,
            row: 0,
            col: 0,
            ..
        })
    ));
}

#[test]
fn verifier_rejects_measure_then_entangle_without_reset() {
    let mut spec = valid_spec();
    spec.layers = vec![
        LayerSpec {
            cycle: 0,
            actions: vec![ActionSpec::Measure {
                atom: 0,
                basis: "z".to_string(),
                duration_us: 10,
            }],
        },
        LayerSpec {
            cycle: 1,
            actions: vec![ActionSpec::Entangle {
                pairs: vec![pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0))],
                duration_us: 1,
            }],
        },
    ];
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::MeasureReuseWithoutReset {
            atom: 0,
            measure_cycle: 0,
            reuse_cycle: 1,
        })
    );
}

#[test]
fn verifier_rejects_measure_and_entangle_same_cycle() {
    let mut spec = valid_spec();
    spec.layers = vec![LayerSpec {
        cycle: 0,
        actions: vec![
            ActionSpec::Measure {
                atom: 0,
                basis: "z".to_string(),
                duration_us: 10,
            },
            ActionSpec::Entangle {
                pairs: vec![pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0))],
                duration_us: 1,
            },
        ],
    }];
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::MeasureUseSameCycle { cycle: 0, atom: 0 })
    );
}

#[test]
fn verifier_rejects_entangle_then_measure_same_cycle() {
    // Order-independent: Use before Measure in the same cycle must still fail.
    let mut spec = valid_spec();
    spec.layers = vec![LayerSpec {
        cycle: 0,
        actions: vec![
            ActionSpec::Entangle {
                pairs: vec![pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0))],
                duration_us: 1,
            },
            ActionSpec::Measure {
                atom: 0,
                basis: "z".to_string(),
                duration_us: 10,
            },
        ],
    }];
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::MeasureUseSameCycle { cycle: 0, atom: 0 })
    );
}

#[test]
fn verifier_rejects_entangle_then_reset_same_cycle() {
    // Order-independent: Use before Reset in the same cycle must still fail.
    let mut spec = valid_spec();
    spec.layers = vec![LayerSpec {
        cycle: 0,
        actions: vec![
            ActionSpec::Entangle {
                pairs: vec![pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0))],
                duration_us: 1,
            },
            ActionSpec::Reset {
                atom: 0,
                duration_us: 10,
            },
        ],
    }];
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::ResetUseSameCycle { cycle: 0, atom: 0 })
    );
}

#[test]
fn verifier_rejects_double_measure_without_reset() {
    let mut spec = valid_spec();
    spec.layers = vec![
        LayerSpec {
            cycle: 0,
            actions: vec![ActionSpec::Measure {
                atom: 0,
                basis: "z".to_string(),
                duration_us: 10,
            }],
        },
        LayerSpec {
            cycle: 1,
            actions: vec![ActionSpec::Measure {
                atom: 0,
                basis: "z".to_string(),
                duration_us: 10,
            }],
        },
    ];
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::DoubleMeasureWithoutReset {
            atom: 0,
            first_cycle: 0,
            second_cycle: 1,
        })
    );
}

#[test]
fn verifier_accepts_reset_then_later_measure() {
    // Reset then a later-cycle measure starts a new round — allowed.
    let mut spec = valid_spec();
    spec.layers = vec![
        LayerSpec {
            cycle: 0,
            actions: vec![ActionSpec::Reset {
                atom: 0,
                duration_us: 10,
            }],
        },
        LayerSpec {
            cycle: 1,
            actions: vec![ActionSpec::Measure {
                atom: 0,
                basis: "z".to_string(),
                duration_us: 10,
            }],
        },
    ];
    assert_eq!(verify_spec(&spec), Ok(()));
}

#[test]
fn verifier_rejects_same_cycle_reset_before_measure() {
    let mut spec = valid_spec();
    spec.layers = vec![LayerSpec {
        cycle: 0,
        actions: vec![
            ActionSpec::Reset {
                atom: 0,
                duration_us: 10,
            },
            ActionSpec::Measure {
                atom: 0,
                basis: "z".to_string(),
                duration_us: 10,
            },
        ],
    }];
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::ResetBeforeMeasure {
            atom: 0,
            reset_cycle: 0,
            measure_cycle: 0,
        })
    );
}

#[test]
fn verifier_rejects_reset_and_entangle_same_cycle() {
    let mut spec = valid_spec();
    spec.layers = vec![LayerSpec {
        cycle: 0,
        actions: vec![
            ActionSpec::Reset {
                atom: 0,
                duration_us: 10,
            },
            ActionSpec::Entangle {
                pairs: vec![pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0))],
                duration_us: 1,
            },
        ],
    }];
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::ResetUseSameCycle { cycle: 0, atom: 0 })
    );
}

#[test]
fn verifier_rejects_layer_after_wait_with_same_cycle() {
    let mut spec = valid_spec();
    spec.layers = vec![
        LayerSpec {
            cycle: 0,
            actions: vec![ActionSpec::Entangle {
                pairs: vec![pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0))],
                duration_us: 1,
            }],
        },
        LayerSpec {
            cycle: 1,
            actions: vec![ActionSpec::Wait { duration_us: 1 }],
        },
        LayerSpec {
            cycle: 1,
            actions: vec![ActionSpec::Entangle {
                pairs: vec![pair(atom(2, 30.0, 0.0), atom(3, 36.0, 0.0))],
                duration_us: 1,
            }],
        },
    ];
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::RoundBarrierCycleOrder {
            wait_cycle: 1,
            after_cycle: 1,
        })
    );
}

#[test]
fn verifier_accepts_measure_reset_wait_then_reuse() {
    let mut spec = valid_spec();
    spec.layers = vec![
        LayerSpec {
            cycle: 0,
            actions: vec![ActionSpec::Measure {
                atom: 1,
                basis: "z".to_string(),
                duration_us: 10,
            }],
        },
        LayerSpec {
            cycle: 1,
            actions: vec![ActionSpec::Reset {
                atom: 1,
                duration_us: 10,
            }],
        },
        LayerSpec {
            cycle: 2,
            actions: vec![ActionSpec::Wait { duration_us: 1 }],
        },
        LayerSpec {
            cycle: 3,
            actions: vec![ActionSpec::Entangle {
                pairs: vec![pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0))],
                duration_us: 1,
            }],
        },
    ];
    assert_eq!(verify_spec(&spec), Ok(()));
}

#[test]
fn verifier_rejects_non_monotonic_cycles() {
    let mut spec = valid_spec();
    spec.layers = vec![
        LayerSpec {
            cycle: 2,
            actions: vec![ActionSpec::Wait { duration_us: 1 }],
        },
        LayerSpec {
            cycle: 1,
            actions: vec![ActionSpec::Wait { duration_us: 1 }],
        },
    ];
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::NonMonotonicCycles {
            previous_cycle: 2,
            cycle: 1,
        })
    );
}

#[test]
fn verifier_rejects_malformed_move_payload() {
    let context = context();
    let location = Location::unknown(&context);
    let op = generic_op(
        &context,
        qna::op::MOVE,
        &[],
        &[],
        &[
            (attr::MOVES, str_attr(&context, "not-json")),
            (attr::DURATION_US, i64_attr(&context, 1)),
        ],
        location,
    );

    assert!(matches!(
        qna::verify(&op),
        Err(VerifyError::JsonAttribute {
            op: qna::op::MOVE,
            attr: attr::MOVES,
            ..
        })
    ));
}

// ===========================================================================
// Issue #282: measure → reset → reuse verifier tests
// ===========================================================================

/// Build a minimal valid schedule with a measure→reset→reuse lifecycle for
/// atom `a` across cycles 0, 1, 2, plus an entangle using the reused atom at
/// cycle 3.
fn reuse_schedule(atom_id: u32) -> ScheduleSpec {
    ScheduleSpec {
        target_id: "generic_reconfigurable_neutral_atom_v0".to_string(),
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
        layers: vec![
            LayerSpec {
                cycle: 0,
                actions: vec![ActionSpec::Measure {
                    atom: atom_id,
                    basis: "z".to_string(),
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 1,
                actions: vec![ActionSpec::Reset {
                    atom: atom_id,
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 2,
                actions: vec![ActionSpec::Reuse {
                    atom: atom_id,
                    region: Some(0),
                    duration_us: 5,
                }],
            },
            LayerSpec {
                cycle: 3,
                actions: vec![ActionSpec::Entangle {
                    pairs: vec![pair(atom(atom_id, 0.0, 0.0), atom(atom_id + 1, 6.0, 0.0))],
                    duration_us: 1,
                }],
            },
        ],
    }
}

#[test]
fn verifier_accepts_legal_reuse_after_measure_reset() {
    // Criterion #5: legal reuse — measure at cycle 0, reset at cycle 1, reuse
    // at cycle 2 (barrier completed across cycle boundaries), then entangle at
    // cycle 3.
    assert_eq!(verify_spec(&reuse_schedule(0)), Ok(()));
}

#[test]
fn verifier_accepts_reuse_without_region_attribute() {
    // `region` is optional; reuse without it must still verify if barriers hold.
    let mut spec = reuse_schedule(0);
    if let ActionSpec::Reuse { region, .. } = &mut spec.layers[2].actions[0] {
        *region = None;
    }
    assert_eq!(verify_spec(&spec), Ok(()));
}

#[test]
fn verifier_rejects_reuse_before_measure() {
    // Criterion #5: reuse-before-measure — reuse an atom that was never
    // measured.
    let spec = ScheduleSpec {
        target_id: "generic_reconfigurable_neutral_atom_v0".to_string(),
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
        layers: vec![LayerSpec {
            cycle: 0,
            actions: vec![ActionSpec::Reuse {
                atom: 0,
                region: Some(0),
                duration_us: 5,
            }],
        }],
    };
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::ReuseBeforeMeasure {
            atom: 0,
            reuse_cycle: 0,
        })
    );
}

#[test]
fn verifier_rejects_reuse_before_reset() {
    // Criterion #5: reuse-before-reset — measure at cycle 0 but reuse at
    // cycle 1 without an intervening reset.
    let spec = ScheduleSpec {
        target_id: "generic_reconfigurable_neutral_atom_v0".to_string(),
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
        layers: vec![
            LayerSpec {
                cycle: 0,
                actions: vec![ActionSpec::Measure {
                    atom: 0,
                    basis: "z".to_string(),
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 1,
                actions: vec![ActionSpec::Reuse {
                    atom: 0,
                    region: Some(0),
                    duration_us: 5,
                }],
            },
        ],
    };
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::ReuseBeforeReset {
            atom: 0,
            measure_cycle: 0,
            reuse_cycle: 1,
        })
    );
}

#[test]
fn verifier_rejects_reuse_same_cycle_as_reset() {
    // Reset barrier has not completed across a cycle boundary — same-cycle
    // reset+reuse must fail as ReuseBeforeReset.
    let spec = ScheduleSpec {
        target_id: "generic_reconfigurable_neutral_atom_v0".to_string(),
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
        layers: vec![
            LayerSpec {
                cycle: 0,
                actions: vec![ActionSpec::Measure {
                    atom: 0,
                    basis: "z".to_string(),
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 1,
                actions: vec![
                    ActionSpec::Reset {
                        atom: 0,
                        duration_us: 10,
                    },
                    ActionSpec::Reuse {
                        atom: 0,
                        region: Some(0),
                        duration_us: 5,
                    },
                ],
            },
        ],
    };
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::ReuseBeforeReset {
            atom: 0,
            measure_cycle: 0,
            reuse_cycle: 1,
        })
    );
}

#[test]
fn verifier_rejects_reuse_after_reset_without_measure() {
    // Reset an atom that was never measured, then reuse it — there is no
    // completed measurement barrier to reuse against.
    let spec = ScheduleSpec {
        target_id: "generic_reconfigurable_neutral_atom_v0".to_string(),
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
        layers: vec![
            LayerSpec {
                cycle: 0,
                actions: vec![ActionSpec::Reset {
                    atom: 0,
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 1,
                actions: vec![ActionSpec::Reuse {
                    atom: 0,
                    region: Some(0),
                    duration_us: 5,
                }],
            },
        ],
    };
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::ReuseBeforeMeasure {
            atom: 0,
            reuse_cycle: 1,
        })
    );
}

#[test]
fn verifier_rejects_stale_measurement_dependency() {
    // Criterion #5: stale-measurement-dependency — measure→reset→reuse at
    // cycles 0/1/2, then a second reuse at cycle 3 without a fresh
    // measure→reset pair.
    let spec = ScheduleSpec {
        target_id: "generic_reconfigurable_neutral_atom_v0".to_string(),
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
        layers: vec![
            LayerSpec {
                cycle: 0,
                actions: vec![ActionSpec::Measure {
                    atom: 0,
                    basis: "z".to_string(),
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 1,
                actions: vec![ActionSpec::Reset {
                    atom: 0,
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 2,
                actions: vec![ActionSpec::Reuse {
                    atom: 0,
                    region: Some(0),
                    duration_us: 5,
                }],
            },
            // Second reuse without fresh measure/reset → stale dependency.
            LayerSpec {
                cycle: 3,
                actions: vec![ActionSpec::Reuse {
                    atom: 0,
                    region: Some(0),
                    duration_us: 5,
                }],
            },
        ],
    };
    assert_eq!(
        verify_spec(&spec),
        Err(VerifyError::StaleMeasurementDependency {
            atom: 0,
            previous_reuse_cycle: 2,
            reuse_cycle: 3,
        })
    );
}

#[test]
fn verifier_accepts_second_reuse_after_fresh_measure_reset() {
    // After reuse, a fresh measure→reset→reuse cycle is legal.
    let spec = ScheduleSpec {
        target_id: "generic_reconfigurable_neutral_atom_v0".to_string(),
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
        layers: vec![
            // First lifecycle.
            LayerSpec {
                cycle: 0,
                actions: vec![ActionSpec::Measure {
                    atom: 0,
                    basis: "z".to_string(),
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 1,
                actions: vec![ActionSpec::Reset {
                    atom: 0,
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 2,
                actions: vec![ActionSpec::Reuse {
                    atom: 0,
                    region: Some(0),
                    duration_us: 5,
                }],
            },
            // Second lifecycle: fresh measure→reset→reuse.
            LayerSpec {
                cycle: 3,
                actions: vec![ActionSpec::Measure {
                    atom: 0,
                    basis: "z".to_string(),
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 4,
                actions: vec![ActionSpec::Reset {
                    atom: 0,
                    duration_us: 10,
                }],
            },
            LayerSpec {
                cycle: 5,
                actions: vec![ActionSpec::Reuse {
                    atom: 0,
                    region: Some(1),
                    duration_us: 5,
                }],
            },
            // Use the reused atom in a later entangle.
            LayerSpec {
                cycle: 6,
                actions: vec![ActionSpec::Entangle {
                    pairs: vec![pair(atom(0, 0.0, 0.0), atom(1, 6.0, 0.0))],
                    duration_us: 1,
                }],
            },
        ],
    };
    assert_eq!(verify_spec(&spec), Ok(()));
}

#[test]
fn verifier_accepts_multi_round_reuse_with_distinct_regions() {
    // Two ancilla regions reused across rounds — each with a full
    // measure→reset→reuse lifecycle.
    let spec = ScheduleSpec {
        target_id: "generic_reconfigurable_neutral_atom_v0".to_string(),
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
        layers: vec![
            // Round 1: measure both ancillae.
            LayerSpec {
                cycle: 0,
                actions: vec![
                    ActionSpec::Measure {
                        atom: 0,
                        basis: "z".to_string(),
                        duration_us: 10,
                    },
                    ActionSpec::Measure {
                        atom: 1,
                        basis: "z".to_string(),
                        duration_us: 10,
                    },
                ],
            },
            // Reset both ancillae.
            LayerSpec {
                cycle: 1,
                actions: vec![
                    ActionSpec::Reset {
                        atom: 0,
                        duration_us: 10,
                    },
                    ActionSpec::Reset {
                        atom: 1,
                        duration_us: 10,
                    },
                ],
            },
            // Reuse both into their respective regions.
            LayerSpec {
                cycle: 2,
                actions: vec![
                    ActionSpec::Reuse {
                        atom: 0,
                        region: Some(0),
                        duration_us: 5,
                    },
                    ActionSpec::Reuse {
                        atom: 1,
                        region: Some(1),
                        duration_us: 5,
                    },
                ],
            },
            // Round 2: measure both again (fresh round).
            LayerSpec {
                cycle: 3,
                actions: vec![
                    ActionSpec::Measure {
                        atom: 0,
                        basis: "z".to_string(),
                        duration_us: 10,
                    },
                    ActionSpec::Measure {
                        atom: 1,
                        basis: "z".to_string(),
                        duration_us: 10,
                    },
                ],
            },
            LayerSpec {
                cycle: 4,
                actions: vec![
                    ActionSpec::Reset {
                        atom: 0,
                        duration_us: 10,
                    },
                    ActionSpec::Reset {
                        atom: 1,
                        duration_us: 10,
                    },
                ],
            },
            LayerSpec {
                cycle: 5,
                actions: vec![
                    ActionSpec::Reuse {
                        atom: 0,
                        region: Some(0),
                        duration_us: 5,
                    },
                    ActionSpec::Reuse {
                        atom: 1,
                        region: Some(1),
                        duration_us: 5,
                    },
                ],
            },
        ],
    };
    assert_eq!(verify_spec(&spec), Ok(()));
}

#[test]
fn verifier_rejects_wrong_float_width_on_schedule_limits() {
    let context = context();
    let location = Location::unknown(&context);
    let region = Region::new();
    region.append_block(Block::new(&[]));
    let schedule = generic_op_with_region(
        &context,
        qna::op::SCHEDULE,
        &[
            (attr::TARGET_ID, str_attr(&context, "target")),
            (attr::RYDBERG_RANGE_UM, f64_attr(&context, 7.5)),
            (attr::MIN_RYDBERG_SPACING_UM, f64_attr(&context, 18.75)),
            (attr::AOD_MIN_SEPARATION_UM, {
                let float_type =
                    Type::parse(&context, "f32").unwrap_or_else(|| Type::none(&context));
                FloatAttribute::new(&context, float_type, 2.0).into()
            }),
        ],
        vec![region],
        location,
    );

    assert_eq!(
        qna::verify(&schedule),
        Err(VerifyError::WrongAttributeType {
            op: qna::op::SCHEDULE,
            attr: attr::AOD_MIN_SEPARATION_UM,
            expected: "f64",
        })
    );
}

fn generic_op_with_region<'c>(
    context: &'c Context,
    name: &str,
    attributes: &[(&str, Attribute<'c>)],
    regions: Vec<Region<'c>>,
    location: Location<'c>,
) -> Operation<'c> {
    let attributes: Vec<(Identifier, Attribute)> = attributes
        .iter()
        .map(|(name, value)| (Identifier::new(context, name), *value))
        .collect();
    OperationBuilder::new(name, location)
        .add_attributes(&attributes)
        .add_regions_vec(regions)
        .build()
        .expect("generic op builds")
}
