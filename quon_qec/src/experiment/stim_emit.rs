//! Lattice-surgery Stim round emitters.
//!
//! [`StimRoundEmitter`] is the extension point for a scheduled [`RoundKind`].
//! [`lattice_round_emitter`] is the only match that selects an impl. The
//! circuit builder walks the schedule and calls [`StimRoundEmitter::emit`];
//! it does not branch on the round kind.
//!
//! A new round kind is a new impl plus one arm in [`lattice_round_emitter`].
//! Single-block memory Stim stays in `experiment.rs` for a later slice.

use std::collections::HashMap;

use crate::expand::{
    ExpandedBlock, ExpandedWorkload, MergeBoundary, PauliFrameUpdate, PhysicalRound, RoundKind,
    RoundTerminal,
};
use crate::workload::LogicalBasis;

use super::{ExperimentError, emit_local_ops, emit_round_body, logical_observable_atoms};

/// Shared Stim state for one lattice-surgery circuit.
///
/// Round impls append instructions and record byproduct handles. Observable
/// assembly after the schedule still lives in the circuit builder.
pub(crate) struct LatticeSurgeryCtx<'a> {
    pub(crate) expanded: &'a ExpandedWorkload,
    pub(crate) control: &'a ExpandedBlock,
    pub(crate) target: &'a ExpandedBlock,
    pub(crate) ancilla: &'a ExpandedBlock,
    pub(crate) out: String,
    pub(crate) byproduct_rec: HashMap<&'static str, i32>,
    pub(crate) rec_count: i32,
    pub(crate) frame_updates: Vec<&'a PauliFrameUpdate>,
    pub(crate) measure_logical_rounds: Vec<&'a PhysicalRound>,
    pub(crate) memory_detector_i: u32,
}

impl<'a> LatticeSurgeryCtx<'a> {
    pub(crate) fn new(
        expanded: &'a ExpandedWorkload,
        control: &'a ExpandedBlock,
        target: &'a ExpandedBlock,
        ancilla: &'a ExpandedBlock,
        out: String,
    ) -> Self {
        Self {
            expanded,
            control,
            target,
            ancilla,
            out,
            byproduct_rec: HashMap::new(),
            rec_count: 0,
            frame_updates: Vec::new(),
            measure_logical_rounds: Vec::new(),
            memory_detector_i: 0,
        }
    }
}

/// Stim instructions for one lattice-surgery round kind.
pub(crate) trait StimRoundEmitter {
    /// Stable registry name. Tests lock [`lattice_round_emitter`] against it.
    #[cfg(test)]
    fn label(&self) -> &'static str;

    /// Append this round's structure Stim to `ctx`.
    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError>;
}

/// Select the impl for `kind`.
///
/// This match is the registry. Emit logic for a kind lives on its impl.
pub(crate) fn lattice_round_emitter(kind: RoundKind) -> &'static dyn StimRoundEmitter {
    match kind {
        RoundKind::Construct => &CONSTRUCT,
        RoundKind::MemoryRound => &MEMORY,
        RoundKind::MeasureLogical => &MEASURE_LOGICAL,
        RoundKind::Merge(MergeBoundary::Rough) => &MERGE_ROUGH,
        RoundKind::Merge(MergeBoundary::Smooth) => &MERGE_SMOOTH,
        RoundKind::Split(_) => &SPLIT,
        RoundKind::MeasureAncilla => &MEASURE_ANCILLA,
        RoundKind::FrameUpdate => &FRAME_UPDATE,
        RoundKind::MagicT => &MAGIC_T,
        RoundKind::MagicTdag => &MAGIC_TDAG,
        RoundKind::MagicCcz => &MAGIC_CCZ,
    }
}

const CONSTRUCT: ConstructEmit = ConstructEmit;
const MEMORY: MemoryRoundEmit = MemoryRoundEmit;
const MEASURE_LOGICAL: MeasureLogicalEmit = MeasureLogicalEmit;
const MERGE_ROUGH: RoughMergeEmit = RoughMergeEmit;
const MERGE_SMOOTH: SmoothMergeEmit = SmoothMergeEmit;
const SPLIT: SplitEmit = SplitEmit;
const MEASURE_ANCILLA: MeasureAncillaEmit = MeasureAncillaEmit;
const FRAME_UPDATE: FrameUpdateEmit = FrameUpdateEmit;
const MAGIC_T: MagicConsumeEmit = MagicConsumeEmit;
const MAGIC_TDAG: MagicConsumeEmit = MagicConsumeEmit;
const MAGIC_CCZ: MagicConsumeEmit = MagicConsumeEmit;

struct ConstructEmit;

impl StimRoundEmitter for ConstructEmit {
    #[cfg(test)]
    fn label(&self) -> &'static str {
        "construct"
    }

    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        emit_local_ops(&mut ctx.out, &round.local_before);
        Ok(())
    }
}

struct MemoryRoundEmit;

impl StimRoundEmitter for MemoryRoundEmit {
    #[cfg(test)]
    fn label(&self) -> &'static str {
        "memory_round"
    }

    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        emit_memory_round(ctx, round)
    }
}

struct MeasureLogicalEmit;

impl StimRoundEmitter for MeasureLogicalEmit {
    #[cfg(test)]
    fn label(&self) -> &'static str {
        "measure_logical"
    }

    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        ctx.measure_logical_rounds.push(round);
        Ok(())
    }
}

struct RoughMergeEmit;

impl StimRoundEmitter for RoughMergeEmit {
    #[cfg(test)]
    fn label(&self) -> &'static str {
        "merge_rough"
    }

    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        _round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        // Logical ZZ(control, ancilla) — one record for the joint parity.
        let control = ctx.control;
        let ancilla = ctx.ancilla;
        let mut ops = Vec::new();
        for a in logical_observable_atoms(control, LogicalBasis::Z) {
            ops.push(format!("Z{a}"));
        }
        for a in logical_observable_atoms(ancilla, LogicalBasis::Z) {
            ops.push(format!("Z{a}"));
        }
        push_mpp(&mut ctx.out, &ops);
        ctx.byproduct_rec.insert("rough_merge", ctx.rec_count);
        ctx.rec_count += 1;
        ctx.out.push_str("TICK\n");
        Ok(())
    }
}

struct SmoothMergeEmit;

impl StimRoundEmitter for SmoothMergeEmit {
    #[cfg(test)]
    fn label(&self) -> &'static str {
        "merge_smooth"
    }

    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        _round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        // Logical XX(ancilla, target).
        let ancilla = ctx.ancilla;
        let target = ctx.target;
        let mut ops = Vec::new();
        for a in logical_observable_atoms(ancilla, LogicalBasis::X) {
            ops.push(format!("X{a}"));
        }
        for a in logical_observable_atoms(target, LogicalBasis::X) {
            ops.push(format!("X{a}"));
        }
        push_mpp(&mut ctx.out, &ops);
        ctx.byproduct_rec.insert("smooth_merge", ctx.rec_count);
        ctx.rec_count += 1;
        ctx.out.push_str("TICK\n");
        Ok(())
    }
}

struct SplitEmit;

impl StimRoundEmitter for SplitEmit {
    #[cfg(test)]
    fn label(&self) -> &'static str {
        "split"
    }

    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        _round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        // NA schedules full stabilizer restore on split; Stim uses logical
        // MPP merges and must not scramble data with mid-protocol MR.
        ctx.out
            .push_str("# split_restore (NA-scheduled; omitted from Stim MPP path)\n");
        Ok(())
    }
}

struct MeasureAncillaEmit;

impl StimRoundEmitter for MeasureAncillaEmit {
    #[cfg(test)]
    fn label(&self) -> &'static str {
        "measure_ancilla"
    }

    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        _round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        // Logical Z of ancilla (top-row product) as one MPP record.
        let ancilla = ctx.ancilla;
        let mut ops = Vec::new();
        for a in logical_observable_atoms(ancilla, LogicalBasis::Z) {
            ops.push(format!("Z{a}"));
        }
        push_mpp(&mut ctx.out, &ops);
        ctx.byproduct_rec.insert("ancilla_mz", ctx.rec_count);
        ctx.rec_count += 1;
        ctx.out.push_str("TICK\n");
        Ok(())
    }
}

struct FrameUpdateEmit;

impl StimRoundEmitter for FrameUpdateEmit {
    #[cfg(test)]
    fn label(&self) -> &'static str {
        "frame_update"
    }

    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        for upd in &round.frame_updates {
            ctx.frame_updates.push(upd);
        }
        Ok(())
    }
}

/// Magic-state consumption (T, T†, CCZ) is one comment shape (issue #283).
struct MagicConsumeEmit;

impl StimRoundEmitter for MagicConsumeEmit {
    #[cfg(test)]
    fn label(&self) -> &'static str {
        "magic_consume"
    }

    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        // Compiler model only — no physical Stim gates.
        ctx.out.push_str(&format!(
            "# {} (magic-state consumption; compiler model, no Stim gate)\n",
            round.kind.as_experiment_str()
        ));
        Ok(())
    }
}

fn push_mpp(out: &mut String, ops: &[String]) {
    out.push_str("MPP ");
    out.push_str(&ops.join("*"));
    out.push('\n');
}

// `trusted` (flux-infer ICE): `.filter_map()`, `.find()`, and `.position()`
// closures in this body trip the same projection ICE as
// `emit_stim_lattice_surgery_cx` (ADR-0027). No flux specs here.
#[cfg_attr(feature = "flux", flux_rs::trusted)]
fn emit_memory_round<'a>(
    ctx: &mut LatticeSurgeryCtx<'a>,
    round: &'a PhysicalRound,
) -> Result<(), ExperimentError> {
    let expanded = ctx.expanded;
    emit_round_body(&mut ctx.out, round)?;
    let measured: Vec<u32> = round
        .terminal
        .iter()
        .filter_map(|t| match t {
            RoundTerminal::Measure { atom, .. } => Some(atom.0),
            _ => None,
        })
        .collect();
    if measured.is_empty() {
        return Ok(());
    }
    ctx.out.push_str("MR");
    for id in &measured {
        ctx.out.push_str(&format!(" {id}"));
        ctx.rec_count += 1;
    }
    ctx.out.push('\n');
    if let Some(block) = expanded
        .blocks
        .iter()
        .find(|b| b.logical_id == round.logical_id)
    {
        let n = measured.len() as i32;
        for stab in &block.stabilizers {
            if stab.basis != LogicalBasis::Z {
                continue;
            }
            let Some(pos) = measured.iter().position(|a| *a == stab.check.0) else {
                continue;
            };
            let cur = -(n - pos as i32);
            let memory_detector_i = ctx.memory_detector_i;
            ctx.out
                .push_str(&format!("DETECTOR({memory_detector_i}, 0) rec[{cur}]\n"));
            ctx.memory_detector_i += 1;
        }
    }
    ctx.out.push_str("TICK\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expand::PhysicalRound;
    use crate::family::{CodeFamily, SourceFamily};
    use crate::workload::{LogicalBasis, LogicalQubitId};

    fn surface_block(id: u32) -> ExpandedBlock {
        ExpandedBlock {
            logical_id: LogicalQubitId(id),
            family: SourceFamily::Surface,
            code_family: CodeFamily::SurfaceCodeLike { distance: 3 },
            distance: 3,
            init_basis: LogicalBasis::Z,
            atoms: Vec::new(),
            data_atoms: Vec::new(),
            check_atoms: Vec::new(),
            coords: Vec::new(),
            stabilizers: Vec::new(),
        }
    }

    fn workload() -> ExpandedWorkload {
        ExpandedWorkload {
            blocks: vec![surface_block(0), surface_block(1), surface_block(2)],
            rounds: Vec::new(),
        }
    }

    fn emit_kind(kind: RoundKind) -> String {
        let expanded = workload();
        let round = PhysicalRound::bare(kind, LogicalQubitId(0));
        let mut ctx = LatticeSurgeryCtx::new(
            &expanded,
            &expanded.blocks[0],
            &expanded.blocks[1],
            &expanded.blocks[2],
            String::new(),
        );
        lattice_round_emitter(kind)
            .emit(&mut ctx, &round)
            .expect("emit");
        ctx.out
    }

    #[test]
    fn every_round_kind_selects_its_emitter() {
        let cases = [
            (RoundKind::Construct, "construct"),
            (RoundKind::MemoryRound, "memory_round"),
            (RoundKind::MeasureLogical, "measure_logical"),
            (RoundKind::Merge(MergeBoundary::Rough), "merge_rough"),
            (RoundKind::Merge(MergeBoundary::Smooth), "merge_smooth"),
            (RoundKind::Split(MergeBoundary::Rough), "split"),
            (RoundKind::Split(MergeBoundary::Smooth), "split"),
            (RoundKind::MeasureAncilla, "measure_ancilla"),
            (RoundKind::FrameUpdate, "frame_update"),
            (RoundKind::MagicT, "magic_consume"),
            (RoundKind::MagicTdag, "magic_consume"),
            (RoundKind::MagicCcz, "magic_consume"),
        ];
        for (kind, label) in cases {
            assert_eq!(lattice_round_emitter(kind).label(), label);
        }
    }

    #[test]
    fn magic_and_split_emitters_write_comments_only() {
        assert_eq!(
            emit_kind(RoundKind::MagicT),
            "# magic_t (magic-state consumption; compiler model, no Stim gate)\n"
        );
        assert_eq!(
            emit_kind(RoundKind::MagicTdag),
            "# magic_tdag (magic-state consumption; compiler model, no Stim gate)\n"
        );
        assert_eq!(
            emit_kind(RoundKind::MagicCcz),
            "# magic_ccz (magic-state consumption; compiler model, no Stim gate)\n"
        );
        assert_eq!(
            emit_kind(RoundKind::Split(MergeBoundary::Rough)),
            "# split_restore (NA-scheduled; omitted from Stim MPP path)\n"
        );
        assert_eq!(
            emit_kind(RoundKind::Split(MergeBoundary::Smooth)),
            emit_kind(RoundKind::Split(MergeBoundary::Rough))
        );
    }

    struct MarkerEmit;

    impl StimRoundEmitter for MarkerEmit {
        #[cfg(test)]
        fn label(&self) -> &'static str {
            "marker"
        }

        fn emit<'a>(
            &self,
            ctx: &mut LatticeSurgeryCtx<'a>,
            _round: &'a PhysicalRound,
        ) -> Result<(), ExperimentError> {
            ctx.out.push_str("# plugged-in\n");
            Ok(())
        }
    }

    #[test]
    fn added_round_emitter_writes_through_the_trait() {
        let expanded = workload();
        let round = PhysicalRound::bare(RoundKind::Construct, LogicalQubitId(0));
        let mut ctx = LatticeSurgeryCtx::new(
            &expanded,
            &expanded.blocks[0],
            &expanded.blocks[1],
            &expanded.blocks[2],
            String::new(),
        );
        MarkerEmit.emit(&mut ctx, &round).expect("custom emitter");
        assert_eq!(ctx.out, "# plugged-in\n");
        assert!(ctx.measure_logical_rounds.is_empty());
    }
}
