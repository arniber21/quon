//! Collect `quantum.dynamic` QEC ops into [`quon_qec::QecWorkload`] (issue #251).

use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, Module, OperationRef, RegionLike};
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
}

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
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

fn collect_op(
    operation: OperationRef<'_, '_>,
    builder: &mut WorkloadBuilder,
) -> Result<(), CollectError> {
    let name = op_name(&operation);
    match name.as_str() {
        op::CONSTRUCT => {
            let family_s = qec_dynamic::read_string_attr(&operation, attr::FAMILY).map_err(|e| {
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
        }
        op::MEMORY_ROUND => {
            let logical_id = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::LOGICAL_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            builder.memory_round(logical_id)?;
        }
        op::MEASURE_LOGICAL => {
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
            builder.measure_logical(logical_id, basis)?;
        }
        op::LOGICAL_CX => {
            let control = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::CONTROL_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            let target = logical_id_from_i64(
                qec_dynamic::read_i64_attr(&operation, attr::TARGET_ID).map_err(|e| {
                    CollectError::InvalidOp {
                        op: name.clone(),
                        detail: e.to_string(),
                    }
                })?,
                &name,
            )?;
            builder.logical_cx(control, target)?;
        }
        _ => {}
    }
    Ok(())
}

fn walk_block(
    block: melior::ir::BlockRef<'_, '_>,
    builder: &mut WorkloadBuilder,
) -> Result<(), CollectError> {
    let mut operation = block.first_operation();
    while let Some(op) = operation {
        collect_op(op, builder)?;
        for region_index in 0..op.region_count() {
            if let Ok(region) = op.region(region_index) {
                let mut inner = region.first_block();
                while let Some(inner_block) = inner {
                    walk_block(inner_block, builder)?;
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
/// Ops are recorded in module textual order. Unsupported family/op combinations
/// surface as [`CollectError::Workload`].
pub fn collect_qec_workload(module: &Module<'_>) -> Result<QecWorkload, CollectError> {
    let mut builder = WorkloadBuilder::new();
    let module_op = module.as_operation();
    let Ok(region) = module_op.region(0) else {
        return Ok(builder.finish());
    };
    let mut block = region.first_block();
    while let Some(current) = block {
        walk_block(current, &mut builder)?;
        block = current.next_in_region();
    }
    Ok(builder.finish())
}

#[cfg(test)]
mod tests {
    use melior::ir::{Location, Value};
    use melior::Context;

    use super::*;
    use crate::dialect::{qec_dynamic, quantum_dynamic};
    use quon_qec::{CodeFamily, LogicalBasis, SourceFamily, WorkloadError, WorkloadOp};

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
                qec_dynamic::qec_measure_logical(context, block, "z", 0, location).expect("measure"),
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
    fn collects_surface_with_logical_cx() {
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
            assert!(matches!(
                workload.ops[2],
                WorkloadOp::LogicalCx {
                    control: LogicalQubitId(0),
                    target: LogicalQubitId(1),
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
                qec_dynamic::qec_construct(context, "surface", 3, "z", 1, location)
                    .expect("surf"),
            );
            let a_v = Value::from(a.result(0).expect("a0"));
            let b_v = Value::from(b.result(0).expect("b0"));
            body.append_operation(
                qec_dynamic::qec_logical_cx(context, a_v, b_v, 0, 1, location).expect("cx"),
            );

            let err = collect_qec_workload(&module).expect_err("should reject");
            assert!(matches!(
                err,
                CollectError::Workload(WorkloadError::LogicalCxFamilyMismatch)
            ));
        });
    }
}
