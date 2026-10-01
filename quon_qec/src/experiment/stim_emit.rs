//! Stim round emitters.
//!
//! [`StimRoundEmitter`] is the extension point for a scheduled [`RoundKind`].
//! [`lattice_round_emitter`] is the only match that selects an impl. Each
//! circuit builder walks the schedule and calls the impl; it does not branch
//! on the round kind.
//!
//! Lattice surgery calls [`StimRoundEmitter::emit`]. Single-block memory calls
//! [`StimRoundEmitter::emit_single_block`] on the same registry. A new round
//! kind is a new impl plus one arm in [`lattice_round_emitter`].
//!
//! Both builders share [`emit_reset_tick`] for the prepare-`R` / `TICK` line,
//! [`emit_qubit_coords`] for `QUBIT_COORDS`, [`emit_measure_line`] for the
//! final `MZ` / `MX` line, [`emit_single_block_header`] /
//! [`emit_lattice_surgery_header`] for the comment preamble, and
//! [`emit_observable_include`] for `OBSERVABLE_INCLUDE` record offsets.
//! Closing detectors stay in the single-block builder. Lattice surgery still
//! appends frame-byproduct records after the shared observable line.

use std::collections::HashMap;

use crate::expand::{
    ExpandedBlock, ExpandedWorkload, MergeBoundary, PauliFrameUpdate, PhysicalAtomId,
    PhysicalRound, RoundKind, RoundTerminal,
};
use crate::workload::LogicalBasis;

use super::{ExperimentError, emit_local_ops, emit_round_body, logical_observable_atoms};

/// Write `R <ids>\nTICK\n` in caller order.
///
/// Single-block memory passes layout order. Lattice surgery passes the sorted
/// unique id list. This function does not reorder.
pub(crate) fn emit_reset_tick(out: &mut String, atom_ids: &[u32]) {
    out.push('R');
    for id in atom_ids {
        out.push_str(&format!(" {id}"));
    }
    out.push_str("\nTICK\n");
}

/// Write `{op} <ids>\n` in caller order.
///
/// Single-block memory passes `MZ` or `MX`. Lattice surgery passes `MZ` and
/// keeps its own record counter. This function does not reorder or skip an
/// empty id list: an empty list is `{op}\n`.
pub(crate) fn emit_measure_line(out: &mut String, op: &str, atom_ids: &[u32]) {
    out.push_str(op);
    for id in atom_ids {
        out.push_str(&format!(" {id}"));
    }
    out.push('\n');
}

/// Write `OBSERVABLE_INCLUDE({obs_id})` and one `rec[-(d - pos)]` per
/// observable atom.
///
/// `data_atoms` is the final measurement window in Stim order. Each
/// `obs_atoms` id must appear in that window; the first match wins. The
/// trailing newline is left to the caller so lattice surgery can append
/// frame-byproduct `rec` targets on the same line.
pub(crate) fn emit_observable_include(
    out: &mut String,
    obs_id: u32,
    data_atoms: &[u32],
    obs_atoms: &[u32],
) -> Result<(), ExperimentError> {
    let d = data_atoms.len() as i32;
    out.push_str(&format!("OBSERVABLE_INCLUDE({obs_id})"));
    for atom in obs_atoms {
        let mut found = None;
        for (pos, id) in data_atoms.iter().enumerate() {
            if id == atom {
                found = Some(pos);
                break;
            }
        }
        let pos = found.ok_or(ExperimentError::MissingDataMeasurement { atom: *atom })?;
        let rec = -(d - pos as i32);
        out.push_str(&format!(" rec[{rec}]"));
    }
    Ok(())
}

/// Write the single-block memory comment preamble.
///
/// The three lines match the checked-in Stim gold, including the surface
/// schedule note on repetition circuits.
pub(crate) fn emit_single_block_header(
    out: &mut String,
    family: &str,
    distance: u32,
    memory_rounds: usize,
    measure_basis: &str,
) {
    out.push_str(&format!(
        "# Quon QEC experiment — structure only (no noise; ADR-0024)\n\
         # family={family} distance={distance} memory_rounds={memory_rounds} measure_basis={measure_basis}\n\
         # Note: surface uses serial Z-then-X expand (not Stim 4-layer FT schedule).\n",
    ));
}

/// Write `QUBIT_COORDS(x, y) id` in caller order.
///
/// Single-block memory passes one block. Lattice surgery calls this once per
/// block, in workload order. Pairs stop at the shorter slice, matching `zip`.
pub(crate) fn emit_qubit_coords(out: &mut String, atoms: &[PhysicalAtomId], coords: &[(i32, i32)]) {
    for (atom, &(x, y)) in atoms.iter().zip(coords.iter()) {
        out.push_str(&format!("QUBIT_COORDS({x}, {y}) {}\n", atom.0));
    }
}

/// Write the lattice-surgery CX comment preamble.
pub(crate) fn emit_lattice_surgery_header(out: &mut String, distance: u32, blocks: usize) {
    out.push_str(&format!(
        "# Quon QEC experiment — lattice-surgery CX structure (no noise; ADR-0019/0024)\n\
         # family=surface distance={distance} blocks={blocks} (L-shaped: control|ancilla / target)\n\
         # Merge/ancilla outcomes → OBSERVABLE_INCLUDE via frame; not bare DETECTORs.\n\
         # Stim merges use logical MPP (Horsman); NA schedules geometric seam CXs.\n\
         # Note: simplified merge/split model; not Stim FT-distance claim.\n",
    ));
}

/// Shared Stim state for one lattice-surgery circuit.
///
/// Round impls append instructions and record byproduct handles. Measure
/// lines, frame-byproduct records, and frame comments stay in the builder.
/// [`emit_observable_include`] writes the shared observable prefix.
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

/// Stim state for one single-block memory circuit.
///
/// The builder still writes closing detectors. [`emit_single_block_header`]
/// writes the comment preamble before this context exists. Round impls append
/// construct locals, memory rounds, and the measure-logical record.
/// [`emit_measure_line`] writes the final measure line.
/// [`emit_observable_include`] writes the observable.
pub(crate) struct SingleBlockCtx<'a> {
    pub(crate) out: String,
    pub(crate) n_checks: usize,
    pub(crate) first_round_detectors: Vec<usize>,
    pub(crate) memory_round_i: usize,
    /// First construct wins, matching the previous `.find()` emitter.
    pub(crate) construct_done: bool,
    pub(crate) measure_logical: Option<&'a PhysicalRound>,
}

impl<'a> SingleBlockCtx<'a> {
    pub(crate) fn new(out: String, n_checks: usize, first_round_detectors: Vec<usize>) -> Self {
        Self {
            out,
            n_checks,
            first_round_detectors,
            memory_round_i: 0,
            construct_done: false,
            measure_logical: None,
        }
    }
}

/// Stim instructions for one round kind.
pub(crate) trait StimRoundEmitter {
    /// Stable registry name. Tests lock [`lattice_round_emitter`] against it.
    #[cfg(test)]
    fn label(&self) -> &'static str;

    /// Append this round's lattice-surgery Stim to `ctx`.
    fn emit<'a>(
        &self,
        ctx: &mut LatticeSurgeryCtx<'a>,
        round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError>;

    /// Append this round's single-block memory Stim to `ctx`.
    ///
    /// Surgery-only kinds leave the circuit unchanged: the single-block builder
    /// used to ignore them.
    fn emit_single_block<'a>(
        &self,
        _ctx: &mut SingleBlockCtx<'a>,
        _round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        Ok(())
    }
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

    fn emit_single_block<'a>(
        &self,
        ctx: &mut SingleBlockCtx<'a>,
        round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        // The builder may call this once before the schedule walk so locals
        // stay ahead of memory rounds. A later construct in the walk is a no-op.
        if ctx.construct_done {
            return Ok(());
        }
        ctx.construct_done = true;
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

    fn emit_single_block<'a>(
        &self,
        ctx: &mut SingleBlockCtx<'a>,
        round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        emit_single_block_memory_round(ctx, round)
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

    fn emit_single_block<'a>(
        &self,
        ctx: &mut SingleBlockCtx<'a>,
        round: &'a PhysicalRound,
    ) -> Result<(), ExperimentError> {
        if ctx.measure_logical.is_none() {
            ctx.measure_logical = Some(round);
        }
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

fn emit_single_block_memory_round(
    ctx: &mut SingleBlockCtx<'_>,
    round: &PhysicalRound,
) -> Result<(), ExperimentError> {
    let round_i = ctx.memory_round_i;
    let n_checks = ctx.n_checks;
    emit_round_body(&mut ctx.out, round)?;
    ctx.out.push_str("MR");
    for term in &round.terminal {
        if let RoundTerminal::Measure { atom, .. } = term {
            ctx.out.push_str(&format!(" {}", atom.0));
        }
    }
    ctx.out.push('\n');

    let detector_indices: Vec<usize> = if round_i == 0 {
        ctx.first_round_detectors.clone()
    } else {
        (0..n_checks).collect()
    };
    for &c in &detector_indices {
        let cur = -(n_checks as i32 - c as i32);
        if round_i == 0 {
            ctx.out
                .push_str(&format!("DETECTOR({c}, {round_i}) rec[{cur}]\n"));
        } else {
            let prev = cur - n_checks as i32;
            ctx.out.push_str(&format!(
                "DETECTOR({c}, {round_i}) rec[{cur}] rec[{prev}]\n"
            ));
        }
    }
    ctx.out.push_str("TICK\n");
    ctx.memory_round_i += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expand::{PhysicalAtomId, PhysicalRound, RoundLocalOp, RoundTerminal};
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

    fn single_block_ctx<'a>() -> SingleBlockCtx<'a> {
        SingleBlockCtx::new(String::new(), 2, vec![0])
    }

    #[test]
    fn single_block_construct_emits_once() {
        let atom = PhysicalAtomId(0);
        let mut round = PhysicalRound::bare(RoundKind::Construct, LogicalQubitId(0));
        round.local_before = vec![RoundLocalOp::H { atom }];
        let mut ctx = single_block_ctx();
        lattice_round_emitter(RoundKind::Construct)
            .emit_single_block(&mut ctx, &round)
            .expect("construct");
        assert_eq!(ctx.out, "H 0\nTICK\n");
        ctx.out.clear();
        lattice_round_emitter(RoundKind::Construct)
            .emit_single_block(&mut ctx, &round)
            .expect("second construct");
        assert_eq!(ctx.out, "");
    }

    #[test]
    fn single_block_memory_round_writes_mr_and_first_detectors() {
        let mut round = PhysicalRound::bare(RoundKind::MemoryRound, LogicalQubitId(0));
        round.terminal = vec![RoundTerminal::Measure {
            atom: PhysicalAtomId(1),
            basis: LogicalBasis::Z,
        }];
        let mut ctx = single_block_ctx();
        lattice_round_emitter(RoundKind::MemoryRound)
            .emit_single_block(&mut ctx, &round)
            .expect("memory");
        assert_eq!(ctx.out, "MR 1\nDETECTOR(0, 0) rec[-2]\nTICK\n");
        assert_eq!(ctx.memory_round_i, 1);
    }

    #[test]
    fn single_block_measure_logical_is_recorded_not_emitted() {
        let round = PhysicalRound::bare(RoundKind::MeasureLogical, LogicalQubitId(0));
        let mut ctx = single_block_ctx();
        lattice_round_emitter(RoundKind::MeasureLogical)
            .emit_single_block(&mut ctx, &round)
            .expect("measure");
        assert_eq!(ctx.out, "");
        assert!(ctx.measure_logical.is_some());
        lattice_round_emitter(RoundKind::MeasureLogical)
            .emit_single_block(&mut ctx, &round)
            .expect("second measure");
        assert!(std::ptr::eq(ctx.measure_logical.expect("first"), &round));
    }

    #[test]
    fn surgery_kinds_leave_single_block_stim_unchanged() {
        let round = PhysicalRound::bare(RoundKind::Split(MergeBoundary::Rough), LogicalQubitId(0));
        let mut ctx = single_block_ctx();
        for kind in [
            RoundKind::Split(MergeBoundary::Rough),
            RoundKind::Merge(MergeBoundary::Smooth),
            RoundKind::MagicT,
            RoundKind::FrameUpdate,
            RoundKind::MeasureAncilla,
        ] {
            lattice_round_emitter(kind)
                .emit_single_block(&mut ctx, &round)
                .expect("silent");
        }
        assert_eq!(ctx.out, "");
        assert!(ctx.measure_logical.is_none());
        assert_eq!(ctx.memory_round_i, 0);
    }

    #[test]
    fn reset_tick_keeps_caller_order() {
        let mut out = String::new();
        emit_reset_tick(&mut out, &[4, 0, 4]);
        assert_eq!(out, "R 4 0 4\nTICK\n");
        let mut empty = String::new();
        emit_reset_tick(&mut empty, &[]);
        assert_eq!(empty, "R\nTICK\n");
    }

    #[test]
    fn observable_include_writes_record_offsets() {
        let mut out = String::new();
        emit_observable_include(&mut out, 0, &[10, 20, 30], &[30, 10]).expect("present");
        assert_eq!(out, "OBSERVABLE_INCLUDE(0) rec[-1] rec[-3]");
        let mut numbered = String::new();
        emit_observable_include(&mut numbered, 1, &[7], &[7]).expect("present");
        assert_eq!(numbered, "OBSERVABLE_INCLUDE(1) rec[-1]");
    }

    #[test]
    fn observable_include_missing_atom_errors() {
        let mut out = String::new();
        let err = emit_observable_include(&mut out, 1, &[10], &[99]).expect_err("missing");
        assert!(matches!(
            err,
            ExperimentError::MissingDataMeasurement { atom: 99 }
        ));
    }

    #[test]
    fn single_block_header_matches_gold_preamble() {
        let mut out = String::new();
        emit_single_block_header(&mut out, "repetition", 3, 2, "z");
        assert_eq!(
            out,
            "# Quon QEC experiment — structure only (no noise; ADR-0024)\n\
             # family=repetition distance=3 memory_rounds=2 measure_basis=z\n\
             # Note: surface uses serial Z-then-X expand (not Stim 4-layer FT schedule).\n"
        );
    }

    #[test]
    fn measure_line_keeps_caller_order() {
        let mut out = String::new();
        emit_measure_line(&mut out, "MZ", &[4, 0, 4]);
        assert_eq!(out, "MZ 4 0 4\n");
        let mut empty = String::new();
        emit_measure_line(&mut empty, "MX", &[]);
        assert_eq!(empty, "MX\n");
    }

    #[test]
    fn qubit_coords_keep_caller_order() {
        let mut out = String::new();
        emit_qubit_coords(
            &mut out,
            &[PhysicalAtomId(4), PhysicalAtomId(0)],
            &[(1, 2), (3, 4)],
        );
        assert_eq!(out, "QUBIT_COORDS(1, 2) 4\nQUBIT_COORDS(3, 4) 0\n");
        let mut short = String::new();
        emit_qubit_coords(&mut short, &[PhysicalAtomId(1)], &[(0, 0), (9, 9)]);
        assert_eq!(short, "QUBIT_COORDS(0, 0) 1\n");
    }

    #[test]
    fn lattice_surgery_header_matches_gold_preamble() {
        let mut out = String::new();
        emit_lattice_surgery_header(&mut out, 3, 3);
        assert_eq!(
            out,
            "# Quon QEC experiment — lattice-surgery CX structure (no noise; ADR-0019/0024)\n\
             # family=surface distance=3 blocks=3 (L-shaped: control|ancilla / target)\n\
             # Merge/ancilla outcomes → OBSERVABLE_INCLUDE via frame; not bare DETECTORs.\n\
             # Stim merges use logical MPP (Horsman); NA schedules geometric seam CXs.\n\
             # Note: simplified merge/split model; not Stim FT-distance claim.\n"
        );
    }
}
