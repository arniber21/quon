//! Collect `quantum.dynamic` QEC ops into [`quon_qec::QecWorkload`] (issue #251).
//!
//! Melior collect site for ADR-0015: frontend/`mlir_bridge` lower QEC builtins
//! into `quantum.dynamic`; this module walks the module and builds MLIR-free
//! workload IR. Wiring into the `quonc` compile pipeline / NA schedule expansion
//! is issue #248.

use std::collections::HashMap;

use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, Module, OperationRef, RegionLike, ValueLike};
use thiserror::Error;

use crate::dialect::qec_dynamic::{self, attr, op};
use quon_qec::{
    LogicalBasis, LogicalQubitId, QecWorkload, SourceFamily, WorkloadBuilder, WorkloadError,
};

/// Failures walking a module for QEC workload IR.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CollectError {
    #[error("QEC workload error: {0}")]
    Workload(#[from] WorkloadError),
    #[error("invalid QEC op `{op}`: {detail}")]
    InvalidOp { op: String, detail: String },
    #[error("`{op}` attribute logical id {attr_id} does not match SSA operand logical id {ssa_id}")]
    LogicalIdMismatch {
        op: String,
        attr_id: u32,
        ssa_id: u32,
    },
    #[error("`{op}`: operand SSA value is not a known QEC block")]
    UnknownSsaBlock { op: String },
}

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
    value.to_raw().ptr as usize
}

fn logical_id_from_i64(value: i64, op: &str) -> Result<LogicalQubitId, CollectError> {
    let id = u32::try_from(value).map_err(|_| CollectError::InvalidOp {
        op: op.to_string(),
        detail: format!("logical id {value} out of u32 range"),
    })?;
    Ok(LogicalQubitId(id))
}

fn distance_from_i64(value: i64, op: &str) -> Result<u32, CollectError> {
    u32::try_from(value).map_err(|_| CollectError::InvalidOp {
        op: op.to_string(),
        detail: format!("distance {value} out of u32 range"),
    })
}

fn resolve_ssa_id(
    operation: OperationRef<'_, '_>,
    operand_index: usize,
    op: &str,
    ssa_ids: &HashMap<usize, LogicalQubitId>,
) -> Result<LogicalQubitId, CollectError> {
    let operand = operation
        .operand(operand_index)
        .map_err(|_| CollectError::InvalidOp {
            op: op.to_string(),
            detail: format!("missing operand {operand_index}"),
        })?;
    ssa_ids
        .get(&value_key(&operand))
        .copied()
        .ok_or_else(|| CollectError::UnknownSsaBlock { op: op.to_string() })
}

fn require_attr_matches_ssa(
    op: &str,
    attr_id: LogicalQubitId,
    ssa_id: LogicalQubitId,
) -> Result<(), CollectError> {
    if attr_id != ssa_id {
        return Err(CollectError::LogicalIdMismatch {
            op: op.to_string(),
            attr_id: attr_id.0,
            ssa_id: ssa_id.0,
        });
    }
    Ok(())
}

fn bind_result(
    operation: OperationRef<'_, '_>,
    result_index: usize,
    logical_id: LogicalQubitId,
    ssa_ids: &mut HashMap<usize, LogicalQubitId>,
    op: &str,
) -> Result<(), CollectError> {
    let result = operation
        .result(result_index)
        .map_err(|_| CollectError::InvalidOp {
            op: op.to_string(),
            detail: format!("missing result {result_index}"),
        })?;
    ssa_ids.insert(value_key(&result), logical_id);
    Ok(())
}

fn collect_op(
    operation: OperationRef<'_, '_>,
    builder: &mut WorkloadBuilder,
    ssa_ids: &mut HashMap<usize, LogicalQubitId>,
) -> Result<(), CollectError> {
    let name = op_name(&operation);
    match name.as_str() {
        op::CONSTRUCT => {
            let family_s =
                qec_dynamic::read_string_attr(&operation, attr::FAMILY).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?;
            let family = SourceFamily::parse(&family_s).ok_or_else(|| CollectError::InvalidOp {
                op: name.clone(),
                detail: format!("unknown family `{family_s}`"),
            })?;
            let distance = distance_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::DISTANCE).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            let basis_s = qec_dynamic::read_string_attr(&operation, attr::BASIS).map_err(|e| {
                CollectError::InvalidOp {
                    op: name.clone(),
                    detail: e.to_string(),
                }
            })?;
            let basis = LogicalBasis::parse(&basis_s).ok_or_else(|| CollectError::InvalidOp {
                op: name.clone(),
                detail: format!("unknown basis `{basis_s}`"),
            })?;
            let logical_id = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::LOGICAL_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            builder.construct(family, distance, basis, logical_id)?;
            bind_result(operation, 0, logical_id, ssa_ids, &name)?;
        }
        op::MEMORY_ROUND => {
            let ssa_id = resolve_ssa_id(operation, 0, &name, ssa_ids)?;
            let attr_id = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::LOGICAL_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            require_attr_matches_ssa(&name, attr_id, ssa_id)?;
            builder.memory_round(ssa_id)?;
            bind_result(operation, 0, ssa_id, ssa_ids, &name)?;
        }
        op::MEASURE_LOGICAL => {
            let ssa_id = resolve_ssa_id(operation, 0, &name, ssa_ids)?;
            let basis_s = qec_dynamic::read_string_attr(&operation, attr::BASIS).map_err(|e| {
                CollectError::InvalidOp {
                    op: name.clone(),
                    detail: e.to_string(),
                }
            })?;
            let basis = LogicalBasis::parse(&basis_s).ok_or_else(|| CollectError::InvalidOp {
                op: name.clone(),
                detail: format!("unknown basis `{basis_s}`"),
            })?;
            let attr_id = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::LOGICAL_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            require_attr_matches_ssa(&name, attr_id, ssa_id)?;
            builder.measure_logical(ssa_id, basis)?;
        }
        op::LOGICAL_CX => {
            let control_ssa = resolve_ssa_id(operation, 0, &name, ssa_ids)?;
            let target_ssa = resolve_ssa_id(operation, 1, &name, ssa_ids)?;
            let control_attr = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::CONTROL_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            let target_attr = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::TARGET_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            require_attr_matches_ssa(&name, control_attr, control_ssa)?;
            require_attr_matches_ssa(&name, target_attr, target_ssa)?;
            builder.logical_cx(control_ssa, target_ssa)?;
            bind_result(operation, 0, control_ssa, ssa_ids, &name)?;
            bind_result(operation, 1, target_ssa, ssa_ids, &name)?;
        }
        op::LOGICAL_T | op::LOGICAL_TDAG => {
            let ssa_id = resolve_ssa_id(operation, 0, &name, ssa_ids)?;
            let attr_id = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::LOGICAL_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            require_attr_matches_ssa(&name, attr_id, ssa_id)?;
            if name == op::LOGICAL_T {
                builder.logical_t(ssa_id)?;
            } else {
                builder.logical_tdag(ssa_id)?;
            }
            bind_result(operation, 0, ssa_id, ssa_ids, &name)?;
        }
        op::LOGICAL_CCZ => {
            let a_ssa = resolve_ssa_id(operation, 0, &name, ssa_ids)?;
            let b_ssa = resolve_ssa_id(operation, 1, &name, ssa_ids)?;
            let c_ssa = resolve_ssa_id(operation, 2, &name, ssa_ids)?;
            let a_attr = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::A_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            let b_attr = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::B_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            let c_attr = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::C_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            require_attr_matches_ssa(&name, a_attr, a_ssa)?;
            require_attr_matches_ssa(&name, b_attr, b_ssa)?;
            require_attr_matches_ssa(&name, c_attr, c_ssa)?;
            builder.logical_ccz(a_ssa, b_ssa, c_ssa)?;
            bind_result(operation, 0, a_ssa, ssa_ids, &name)?;
            bind_result(operation, 1, b_ssa, ssa_ids, &name)?;
            bind_result(operation, 2, c_ssa, ssa_ids, &name)?;
        }
        other if other.starts_with("quantum.dynamic.qec_") => {
            return Err(CollectError::InvalidOp {
                op: name,
                detail: "unrecognized QEC dynamic op".into(),
            });
        }
        _ => {}
    }
    Ok(())
}

fn walk_block(
    block: melior::ir::BlockRef<'_, '_>,
    builder: &mut WorkloadBuilder,
    ssa_ids: &mut HashMap<usize, LogicalQubitId>,
) -> Result<(), CollectError> {
    let mut operation = block.first_operation();
    while let Some(op) = operation {
        collect_op(op, builder, ssa_ids)?;
        for region_index in 0..op.region_count() {
            if let Ok(region) = op.region(region_index) {
                let mut inner = region.first_block();
                while let Some(inner_block) = inner {
                    walk_block(inner_block, builder, ssa_ids)?;
                    inner = inner_block.next_in_region();
                }
            }
        }
        operation = op.next_in_block();
    }
    Ok(())
}

/// Walk a `quantum.dynamic` module and collect QEC ops into a [`QecWorkload`].
///
/// Ops are recorded in module textual order. Logical ids are taken from SSA
/// result→id bindings established at `qec_construct` (and threaded through
/// round/CX results); attributes must match those SSA ids. Unsupported
/// family/op combinations surface as [`CollectError::Workload`].
pub fn collect_qec_workload(module: &Module<'_>) -> Result<QecWorkload, CollectError> {
    let mut builder = WorkloadBuilder::new();
    let mut ssa_ids = HashMap::new();
    let module_op = module.as_operation();
    let Ok(region) = module_op.region(0) else {
        return Ok(builder.finish());
    };
    let mut block = region.first_block();
    while let Some(current) = block {
        walk_block(current, &mut builder, &mut ssa_ids)?;
        block = current.next_in_region();
    }
    Ok(builder.finish())
}

#[cfg(test)]
mod tests {
    use melior::Context;
    use melior::ir::{Location, Value};

    use super::*;
    use crate::dialect::{qec_dynamic, quantum_dynamic};
    use quon_qec::{
        CodeFamily, LogicalBasis, LogicalQubitId, SourceFamily, WorkloadError, WorkloadOp,
    };

    fn with_ctx<F>(f: F)
    where
        F: FnOnce(&Context),
    {
        let context = Context::new();
        context.set_allow_unregistered_dialects(true);
        quantum_dynamic::register_dialect(&context);
        f(&context);
    }

    #[test]
    fn collects_repetition_memory_order_and_metadata() {
        with_ctx(|context| {
            let location = Location::unknown(context);
            let module = melior::ir::Module::new(location);
            let body = module.body();

            let c = body.append_operation(
                qec_dynamic::qec_construct(context, "repetition", 3, "z", 0, location)
                    .expect("construct"),
            );
            let block = Value::from(c.result(0).expect("r0"));
            let r1 = body.append_operation(
                qec_dynamic::qec_memory_round(context, block, 0, location).expect("round1"),
            );
            let block = Value::from(r1.result(0).expect("r1"));
            let r2 = body.append_operation(
                qec_dynamic::qec_memory_round(context, block, 0, location).expect("round2"),
            );
            let block = Value::from(r2.result(0).expect("r2"));
            body.append_operation(
                qec_dynamic::qec_measure_logical(context, block, "z", 0, location)
                    .expect("measure"),
            );

            let workload = collect_qec_workload(&module).expect("collect");
            assert_eq!(workload.blocks.len(), 1);
            assert_eq!(workload.blocks[0].family, SourceFamily::Repetition);
            assert_eq!(workload.blocks[0].distance, 3);
            assert_eq!(
                workload.blocks[0].code_family,
                CodeFamily::RepetitionCodeToy { distance: 3 }
            );
            assert_eq!(workload.memory_round_count(), 2);
            assert_eq!(
                workload.ops.last(),
                Some(&WorkloadOp::MeasureLogical {
                    logical_id: LogicalQubitId(0),
                    basis: LogicalBasis::Z,
                })
            );
        });
    }

    #[test]
    fn collects_surface_with_logical_cx_full_order_and_metadata() {
        with_ctx(|context| {
            let location = Location::unknown(context);
            let module = melior::ir::Module::new(location);
            let body = module.body();

            let a = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 3, "z", 0, location).expect("a"),
            );
            let b = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 3, "x", 1, location).expect("b"),
            );
            let a_v = Value::from(a.result(0).expect("a0"));
            let b_v = Value::from(b.result(0).expect("b0"));
            let cx = body.append_operation(
                qec_dynamic::qec_logical_cx(context, a_v, b_v, 0, 1, location).expect("cx"),
            );
            let a2 = Value::from(cx.result(0).expect("cx0"));
            let b2 = Value::from(cx.result(1).expect("cx1"));
            body.append_operation(
                qec_dynamic::qec_measure_logical(context, a2, "z", 0, location).expect("mz"),
            );
            body.append_operation(
                qec_dynamic::qec_measure_logical(context, b2, "x", 1, location).expect("mx"),
            );

            let workload = collect_qec_workload(&module).expect("collect");
            assert_eq!(workload.blocks.len(), 2);
            let block0 = &workload.blocks[0];
            assert_eq!(block0.family, SourceFamily::Surface);
            assert_eq!(block0.distance, 3);
            assert_eq!(block0.init_basis, LogicalBasis::Z);
            assert_eq!(
                block0.code_family,
                CodeFamily::SurfaceCodeLike { distance: 3 }
            );
            let block1 = &workload.blocks[1];
            assert_eq!(block1.family, SourceFamily::Surface);
            assert_eq!(block1.distance, 3);
            assert_eq!(block1.init_basis, LogicalBasis::X);
            assert_eq!(
                block1.code_family,
                CodeFamily::SurfaceCodeLike { distance: 3 }
            );
            assert_eq!(
                workload.ops,
                vec![
                    WorkloadOp::Construct {
                        family: SourceFamily::Surface,
                        distance: 3,
                        basis: LogicalBasis::Z,
                        logical_id: LogicalQubitId(0),
                    },
                    WorkloadOp::Construct {
                        family: SourceFamily::Surface,
                        distance: 3,
                        basis: LogicalBasis::X,
                        logical_id: LogicalQubitId(1),
                    },
                    WorkloadOp::LogicalCx {
                        control: LogicalQubitId(0),
                        target: LogicalQubitId(1),
                    },
                    WorkloadOp::MeasureLogical {
                        logical_id: LogicalQubitId(0),
                        basis: LogicalBasis::Z,
                    },
                    WorkloadOp::MeasureLogical {
                        logical_id: LogicalQubitId(1),
                        basis: LogicalBasis::X,
                    },
                ]
            );
        });
    }

    #[test]
    fn rejects_attr_ssa_logical_id_mismatch() {
        with_ctx(|context| {
            let location = Location::unknown(context);
            let module = melior::ir::Module::new(location);
            let body = module.body();

            let c = body.append_operation(
                qec_dynamic::qec_construct(context, "repetition", 3, "z", 0, location)
                    .expect("construct"),
            );
            let block = Value::from(c.result(0).expect("r0"));
            // Operand is block 0, but attribute claims logical_id=1.
            body.append_operation(
                qec_dynamic::qec_memory_round(context, block, 1, location).expect("round"),
            );

            let err = collect_qec_workload(&module).expect_err("mismatch");
            assert!(matches!(
                err,
                CollectError::LogicalIdMismatch {
                    attr_id: 1,
                    ssa_id: 0,
                    ..
                }
            ));
        });
    }

    #[test]
    fn unsupported_logical_cx_on_repetition_is_diagnostic() {
        with_ctx(|context| {
            let location = Location::unknown(context);
            let module = melior::ir::Module::new(location);
            let body = module.body();

            let a = body.append_operation(
                qec_dynamic::qec_construct(context, "repetition", 3, "z", 0, location)
                    .expect("rep"),
            );
            let b = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 3, "z", 1, location).expect("surf"),
            );
            let a_v = Value::from(a.result(0).expect("a0"));
            let b_v = Value::from(b.result(0).expect("b0"));
            body.append_operation(
                qec_dynamic::qec_logical_cx(context, a_v, b_v, 0, 1, location).expect("cx"),
            );

            let err = collect_qec_workload(&module).expect_err("should reject");
            assert!(matches!(
                err,
                CollectError::Workload(WorkloadError::LogicalCxNotSurface {
                    id: 0,
                    family: "repetition",
                })
            ));
        });
    }

    #[test]
    fn verify_rejects_repetition_x_and_invalid_distance() {
        with_ctx(|context| {
            let location = Location::unknown(context);
            assert!(
                qec_dynamic::qec_construct(context, "repetition", 3, "x", 0, location).is_err(),
                "repetition + x must fail at build/verify"
            );
            assert!(
                qec_dynamic::qec_construct(context, "surface", 2, "z", 0, location).is_err(),
                "even surface distance must fail at build/verify"
            );
        });
    }

    #[test]
    fn rejects_use_after_measure_memory_round() {
        with_ctx(|context| {
            let location = Location::unknown(context);
            let module = melior::ir::Module::new(location);
            let body = module.body();

            let c = body.append_operation(
                qec_dynamic::qec_construct(context, "repetition", 3, "z", 0, location)
                    .expect("construct"),
            );
            let block = Value::from(c.result(0).expect("r0"));
            body.append_operation(
                qec_dynamic::qec_measure_logical(context, block, "z", 0, location)
                    .expect("measure"),
            );
            // Reuse the pre-measure SSA value (builder-level use-after-measure).
            body.append_operation(
                qec_dynamic::qec_memory_round(context, block, 0, location).expect("round"),
            );

            let err = collect_qec_workload(&module).expect_err("use-after-measure");
            assert!(matches!(
                err,
                CollectError::Workload(WorkloadError::UseAfterMeasure(0))
            ));
        });
    }

    #[test]
    fn rejects_use_after_measure_logical_cx() {
        with_ctx(|context| {
            let location = Location::unknown(context);
            let module = melior::ir::Module::new(location);
            let body = module.body();

            let a = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 3, "z", 0, location).expect("a"),
            );
            let b = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 3, "z", 1, location).expect("b"),
            );
            let a_v = Value::from(a.result(0).expect("a0"));
            let b_v = Value::from(b.result(0).expect("b0"));
            body.append_operation(
                qec_dynamic::qec_measure_logical(context, a_v, "z", 0, location).expect("mz"),
            );
            body.append_operation(
                qec_dynamic::qec_logical_cx(context, a_v, b_v, 0, 1, location).expect("cx"),
            );

            let err = collect_qec_workload(&module).expect_err("use-after-measure");
            assert!(matches!(
                err,
                CollectError::Workload(WorkloadError::UseAfterMeasure(0))
            ));
        });
    }

    #[test]
    fn collects_surface_logical_t_and_ccz_full_order() {
        with_ctx(|context| {
            let location = Location::unknown(context);
            let module = melior::ir::Module::new(location);
            let body = module.body();

            let a = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 3, "z", 0, location).expect("a"),
            );
            let b = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 3, "z", 1, location).expect("b"),
            );
            let c = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 3, "z", 2, location).expect("c"),
            );
            let a_v = Value::from(a.result(0).expect("a0"));
            let b_v = Value::from(b.result(0).expect("b0"));
            let c_v = Value::from(c.result(0).expect("c0"));
            // logical_t consumes a magic state, returns the same block (stays live).
            let t = body.append_operation(
                qec_dynamic::qec_logical_t(context, a_v, 0, location).expect("t"),
            );
            let a_t = Value::from(t.result(0).expect("t0"));
            // logical_ccz over three live surface blocks at equal distance.
            let ccz = body.append_operation(
                qec_dynamic::qec_logical_ccz(context, a_t, b_v, c_v, 0, 1, 2, location)
                    .expect("ccz"),
            );
            let a2 = Value::from(ccz.result(0).expect("ccz0"));
            let b2 = Value::from(ccz.result(1).expect("ccz1"));
            let c2 = Value::from(ccz.result(2).expect("ccz2"));
            body.append_operation(
                qec_dynamic::qec_measure_logical(context, a2, "z", 0, location).expect("mza"),
            );
            body.append_operation(
                qec_dynamic::qec_measure_logical(context, b2, "z", 1, location).expect("mzb"),
            );
            body.append_operation(
                qec_dynamic::qec_measure_logical(context, c2, "z", 2, location).expect("mzc"),
            );

            let workload = collect_qec_workload(&module).expect("collect");
            assert_eq!(workload.blocks.len(), 3);
            assert_eq!(
                workload.ops,
                vec![
                    WorkloadOp::Construct {
                        family: SourceFamily::Surface,
                        distance: 3,
                        basis: LogicalBasis::Z,
                        logical_id: LogicalQubitId(0),
                    },
                    WorkloadOp::Construct {
                        family: SourceFamily::Surface,
                        distance: 3,
                        basis: LogicalBasis::Z,
                        logical_id: LogicalQubitId(1),
                    },
                    WorkloadOp::Construct {
                        family: SourceFamily::Surface,
                        distance: 3,
                        basis: LogicalBasis::Z,
                        logical_id: LogicalQubitId(2),
                    },
                    WorkloadOp::LogicalT {
                        logical_id: LogicalQubitId(0),
                    },
                    WorkloadOp::LogicalCcz {
                        a: LogicalQubitId(0),
                        b: LogicalQubitId(1),
                        c: LogicalQubitId(2),
                    },
                    WorkloadOp::MeasureLogical {
                        logical_id: LogicalQubitId(0),
                        basis: LogicalBasis::Z,
                    },
                    WorkloadOp::MeasureLogical {
                        logical_id: LogicalQubitId(1),
                        basis: LogicalBasis::Z,
                    },
                    WorkloadOp::MeasureLogical {
                        logical_id: LogicalQubitId(2),
                        basis: LogicalBasis::Z,
                    },
                ]
            );
        });
    }

    #[test]
    fn unsupported_logical_t_on_repetition_is_diagnostic() {
        with_ctx(|context| {
            let location = Location::unknown(context);
            let module = melior::ir::Module::new(location);
            let body = module.body();

            let a = body.append_operation(
                qec_dynamic::qec_construct(context, "repetition", 3, "z", 0, location)
                    .expect("rep"),
            );
            let a_v = Value::from(a.result(0).expect("a0"));
            body.append_operation(
                qec_dynamic::qec_logical_t(context, a_v, 0, location).expect("t"),
            );

            let err = collect_qec_workload(&module).expect_err("should reject");
            assert!(matches!(
                err,
                CollectError::Workload(WorkloadError::NonCliffordNotSurface {
                    op: "logical_t",
                    id: 0,
                    family: "repetition",
                })
            ));
        });
    }

    #[test]
    fn unsupported_logical_ccz_distance_mismatch_is_diagnostic() {
        with_ctx(|context| {
            let location = Location::unknown(context);
            let module = melior::ir::Module::new(location);
            let body = module.body();

            let a = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 3, "z", 0, location).expect("a"),
            );
            let b = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 5, "z", 1, location).expect("b"),
            );
            let c = body.append_operation(
                qec_dynamic::qec_construct(context, "surface", 5, "z", 2, location).expect("c"),
            );
            let a_v = Value::from(a.result(0).expect("a0"));
            let b_v = Value::from(b.result(0).expect("b0"));
            let c_v = Value::from(c.result(0).expect("c0"));
            body.append_operation(
                qec_dynamic::qec_logical_ccz(context, a_v, b_v, c_v, 0, 1, 2, location)
                    .expect("ccz"),
            );

            let err = collect_qec_workload(&module).expect_err("should reject");
            assert!(matches!(
                err,
                CollectError::Workload(WorkloadError::LogicalCczDistanceMismatch)
            ));
        });
    }
}
