//! Fixed-target mapping trace wire types (issue #135).
//!
//! SABRE records a [`MappingLog`] while it inserts SWAPs. [`assemble_mapping_trace`]
//! turns that log plus per-stage metric snapshots into the versioned
//! [`MappingTrace`] JSON that `quonc --emit-mapping-json` writes. The collector
//! stays in `mlir_bridge` (it walks IR); this module is the MLIR-free DTO and
//! the narration, so schema tests do not link LLVM. Same split as
//! [`crate::metrics`].

use serde::{Deserialize, Serialize};

/// Current [`MappingTrace`] schema version.
pub const MAPPING_TRACE_VERSION: u32 = 1;

/// Stable wire kind for mapping visualization JSON.
pub const MAPPING_TRACE_KIND: &str = "mapping_trace";

/// Stage id: placement before any routing SWAP.
pub const STAGE_LAYOUT: &str = "layout";

/// Stage id: SABRE routing, including inserted SWAP ops.
pub const STAGE_ROUTING: &str = "routing";

/// Stage id: after post-SWAP native decomposition and depth scheduling.
pub const STAGE_NATIVE_DECOMP: &str = "native_decomp";

/// In-memory routing record. Not a wire type — [`assemble_mapping_trace`]
/// fills human `summary` strings before serialization.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MappingLog {
    /// Logical → physical pairs in first-use order, captured at assignment
    /// time (before any SWAP moves that qubit).
    pub initial: Vec<(u64, u64)>,
    pub events: Vec<RawMappingEvent>,
    /// Logical → physical pairs after routing. Sorted by logical id when
    /// [`MappingLog::set_final`] is used.
    pub final_layout: Vec<(u64, u64)>,
}

/// Which arm of a `quantum.dynamic.if` an event list belongs to.
///
/// Then is region 0 (condition bit set). Else is region 1 (bit clear).
/// Exactly one arm runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchArm {
    Then,
    Else,
}

/// One routing action, before narration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawMappingEvent {
    Swap {
        logical: [u64; 2],
        physical: [u64; 2],
    },
    Interaction {
        gate: String,
        logical: [u64; 2],
        physical: [u64; 2],
    },
    /// Events for one arm of a measurement branch. The sibling arm is a
    /// separate alternative; the two lists are not one execution.
    /// `layout` is that arm's logical → physical map after the arm finishes.
    Branch {
        arm: BranchArm,
        events: Vec<RawMappingEvent>,
        layout: Vec<(u64, u64)>,
    },
}

impl MappingLog {
    /// Records the physical index a logical qubit received on first placement.
    pub fn note_initial(&mut self, logical: u64, physical: u64) {
        if self
            .initial
            .iter()
            .any(|(existing, _)| *existing == logical)
        {
            return;
        }
        self.initial.push((logical, physical));
    }

    /// Records a SWAP SABRE actually inserted.
    pub fn note_swap(&mut self, logical: [u64; 2], physical: [u64; 2]) {
        self.events
            .push(RawMappingEvent::Swap { logical, physical });
    }

    /// Records a two-qubit gate after it was moved onto a coupling edge.
    pub fn note_interaction(&mut self, gate: String, logical: [u64; 2], physical: [u64; 2]) {
        self.events.push(RawMappingEvent::Interaction {
            gate,
            logical,
            physical,
        });
    }

    /// Stores the post-routing layout, sorted by logical id so JSON is stable.
    pub fn set_final(&mut self, mut pairs: Vec<(u64, u64)>) {
        pairs.sort_by_key(|(logical, _)| *logical);
        self.final_layout = pairs;
    }

    /// Appends one measurement-branch arm and the layout that arm finished on.
    ///
    /// An empty arm is still recorded so the trace shows the alternative
    /// instead of implying the other arm always runs.
    pub fn note_branch(
        &mut self,
        arm: BranchArm,
        events: Vec<RawMappingEvent>,
        layout: Vec<(u64, u64)>,
    ) {
        self.events.push(RawMappingEvent::Branch {
            arm,
            events,
            layout,
        });
    }

    /// Number of recorded SWAP insertions, including those inside branch arms.
    pub fn swap_count(&self) -> usize {
        count_swaps(&self.events)
    }
}

fn count_swaps(events: &[RawMappingEvent]) -> usize {
    events
        .iter()
        .map(|event| match event {
            RawMappingEvent::Swap { .. } => 1,
            RawMappingEvent::Interaction { .. } => 0,
            RawMappingEvent::Branch { events, .. } => count_swaps(events),
        })
        .sum()
}

/// Inputs to [`assemble_mapping_trace`]. Metric snapshots are taken by the
/// caller around SABRE; this function does not inspect IR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MappingTraceParts {
    pub target_id: String,
    pub edges: Vec<(u64, u64)>,
    pub log: MappingLog,
    pub before_routing: MappingStageMetrics,
    pub after_routing: MappingStageMetrics,
    pub final_metrics: MappingStageMetrics,
}

/// `#48`-shaped counts embedded in each mapping stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingStageMetrics {
    pub gate_count: u64,
    pub depth: u64,
    pub swap_count: u64,
    pub t_count: u64,
}

/// Target identity carried on the trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingTraceMeta {
    pub target_id: String,
}

/// Coupling edges the router was given. Each edge is ordered `(lo, hi)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingTopology {
    pub edges: Vec<[u64; 2]>,
}

/// One logical qubit's physical home.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QubitAssignment {
    pub logical: u64,
    pub physical: u64,
}

/// A narrated routing event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MappingEvent {
    Swap {
        logical: [u64; 2],
        physical: [u64; 2],
        summary: String,
    },
    Interaction {
        gate: String,
        logical: [u64; 2],
        physical: [u64; 2],
        summary: String,
    },
    /// One arm of a `quantum.dynamic.if`. The sibling arm is a separate event,
    /// not the next step of this one. `layout` is the permutation that arm
    /// finished on.
    Branch {
        arm: BranchArm,
        summary: String,
        events: Vec<MappingEvent>,
        layout: Vec<QubitAssignment>,
    },
}

/// One pipeline stage the viewer can scrub to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingStage {
    pub id: String,
    pub summary: String,
    pub metrics: MappingStageMetrics,
}

/// Versioned mapping document emitted by `--emit-mapping-json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingTrace {
    pub schema_version: u32,
    pub kind: String,
    pub summary: String,
    pub meta: MappingTraceMeta,
    pub topology: MappingTopology,
    pub initial_layout: Vec<QubitAssignment>,
    pub final_layout: Vec<QubitAssignment>,
    pub events: Vec<MappingEvent>,
    pub stages: Vec<MappingStage>,
}

impl MappingTrace {
    /// Pretty-printed JSON for CLI emit.
    pub fn to_json_string_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Builds a v1 trace. Summaries are compile-time narration, not viewer guesses.
pub fn assemble_mapping_trace(parts: MappingTraceParts) -> MappingTrace {
    let events = narrate_events(&parts.log.events);
    let summary = routing_summary(&parts.log);
    let layout_summary = layout_summary(&parts.log.initial);
    let routing_stage_summary = format!(
        "{summary} {}",
        metrics_delta(&parts.before_routing, &parts.after_routing)
    );
    let decomp_summary = decomp_summary(&parts.log, &parts.after_routing, &parts.final_metrics);
    MappingTrace {
        schema_version: MAPPING_TRACE_VERSION,
        kind: MAPPING_TRACE_KIND.to_string(),
        summary,
        meta: MappingTraceMeta {
            target_id: parts.target_id,
        },
        topology: MappingTopology {
            edges: canon_edges(&parts.edges),
        },
        initial_layout: assignments(&parts.log.initial),
        final_layout: assignments_sorted(&parts.log.final_layout),
        events,
        stages: vec![
            MappingStage {
                id: STAGE_LAYOUT.to_string(),
                summary: layout_summary,
                metrics: parts.before_routing,
            },
            MappingStage {
                id: STAGE_ROUTING.to_string(),
                summary: routing_stage_summary,
                metrics: parts.after_routing,
            },
            MappingStage {
                id: STAGE_NATIVE_DECOMP.to_string(),
                summary: decomp_summary,
                metrics: parts.final_metrics,
            },
        ],
    }
}

fn assignments(pairs: &[(u64, u64)]) -> Vec<QubitAssignment> {
    pairs
        .iter()
        .map(|(logical, physical)| QubitAssignment {
            logical: *logical,
            physical: *physical,
        })
        .collect()
}

fn assignments_sorted(pairs: &[(u64, u64)]) -> Vec<QubitAssignment> {
    let mut pairs = pairs.to_vec();
    pairs.sort_by_key(|(logical, _)| *logical);
    assignments(&pairs)
}

fn canon_edges(edges: &[(u64, u64)]) -> Vec<[u64; 2]> {
    let mut out: Vec<[u64; 2]> = edges
        .iter()
        .map(|(left, right)| {
            if left <= right {
                [*left, *right]
            } else {
                [*right, *left]
            }
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn narrate_events(events: &[RawMappingEvent]) -> Vec<MappingEvent> {
    events
        .iter()
        .map(|event| match event {
            RawMappingEvent::Swap { logical, physical } => MappingEvent::Swap {
                logical: *logical,
                physical: *physical,
                summary: format!(
                    "Sabre inserted a SWAP on physical edge ({}, {}), exchanging logical qubits {} and {}.",
                    physical[0], physical[1], logical[0], logical[1]
                ),
            },
            RawMappingEvent::Interaction {
                gate,
                logical,
                physical,
            } => MappingEvent::Interaction {
                gate: gate.clone(),
                logical: *logical,
                physical: *physical,
                summary: format!(
                    "{gate} on logical qubits ({}, {}) executes on physical qubits ({}, {}).",
                    logical[0], logical[1], physical[0], physical[1]
                ),
            },
            RawMappingEvent::Branch {
                arm,
                events,
                layout,
            } => MappingEvent::Branch {
                arm: *arm,
                summary: branch_summary(*arm, events),
                events: narrate_events(events),
                layout: assignments(layout),
            },
        })
        .collect()
}

fn branch_summary(arm: BranchArm, events: &[RawMappingEvent]) -> String {
    let (name, other) = match arm {
        BranchArm::Then => ("Then", "else"),
        BranchArm::Else => ("Else", "then"),
    };
    let swaps = count_swaps(events);
    format!(
        "{name} arm of a measurement branch ({swaps} SWAP insertion(s) on this arm only). The {other} arm is an alternative and does not also run."
    )
}

fn routing_summary(log: &MappingLog) -> String {
    let swaps = top_level_swaps(&log.events);
    let sentence = if swaps == 0 && !has_branch(&log.events) {
        "Sabre inserted no SWAPs; every two-qubit interaction was already on a coupling edge."
            .to_string()
    } else if swaps == 0 {
        "Outside measurement branches, Sabre inserted no SWAPs.".to_string()
    } else {
        swap_sentence(&log.events, swaps)
    };
    if has_branch(&log.events) {
        let arms = "Each measurement branch lists its then and else arms separately; exactly one arm runs.";
        if arm_layouts_disagree(&log.events) {
            format!(
                "{sentence} {arms} The arms finish on different permutations, so there is no single post-branch layout and later gates are not routed against the pre-branch map."
            )
        } else {
            format!("{sentence} {arms}")
        }
    } else {
        sentence
    }
}

/// True when a then/else pair finished on different permutations.
fn arm_layouts_disagree(events: &[RawMappingEvent]) -> bool {
    let mut rest = events;
    while let Some((event, tail)) = rest.split_first() {
        match event {
            RawMappingEvent::Branch {
                arm: BranchArm::Then,
                events: then_events,
                layout: then_layout,
            } => {
                if arm_layouts_disagree(then_events) {
                    return true;
                }
                if let Some((
                    RawMappingEvent::Branch {
                        arm: BranchArm::Else,
                        events: else_events,
                        layout: else_layout,
                    },
                    after_else,
                )) = tail.split_first()
                {
                    if then_layout != else_layout || arm_layouts_disagree(else_events) {
                        return true;
                    }
                    rest = after_else;
                    continue;
                }
            }
            RawMappingEvent::Branch { events: nested, .. } => {
                if arm_layouts_disagree(nested) {
                    return true;
                }
            }
            RawMappingEvent::Swap { .. } | RawMappingEvent::Interaction { .. } => {}
        }
        rest = tail;
    }
    false
}

fn has_branch(events: &[RawMappingEvent]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, RawMappingEvent::Branch { .. }))
}

fn top_level_swaps(events: &[RawMappingEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, RawMappingEvent::Swap { .. }))
        .count()
}

fn swap_sentence(events: &[RawMappingEvent], swaps: usize) -> String {
    let mut edges: Vec<[u64; 2]> = events
        .iter()
        .filter_map(|event| match event {
            RawMappingEvent::Swap { physical, .. } => Some(canon_pair(*physical)),
            RawMappingEvent::Interaction { .. } | RawMappingEvent::Branch { .. } => None,
        })
        .collect();
    edges.sort_unstable();
    edges.dedup();
    let listed = edges
        .iter()
        .map(|edge| format!("({}, {})", edge[0], edge[1]))
        .collect::<Vec<_>>()
        .join(", ");
    let noun = if swaps == 1 { "SWAP" } else { "SWAPs" };
    format!("Sabre inserted {swaps} {noun} on physical edges {listed}.")
}

fn layout_summary(initial: &[(u64, u64)]) -> String {
    if initial.is_empty() {
        return "No logical qubits were placed.".to_string();
    }
    let pairs = initial
        .iter()
        .map(|(logical, physical)| format!("{logical}→{physical}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Initial layout places {} logical qubits in first-use order: {pairs}.",
        initial.len()
    )
}

fn decomp_summary(
    log: &MappingLog,
    after_routing: &MappingStageMetrics,
    final_metrics: &MappingStageMetrics,
) -> String {
    let delta = metrics_delta(after_routing, final_metrics);
    if after_routing.swap_count > 0 && final_metrics.swap_count == 0 {
        format!(
            "Native decomposition rewrote {} literal SWAP gate(s) into CX triples before final metrics, so swap_count dropped from {} to 0. Trust the event list ({} SWAP insertion(s)), not the final counter. {delta}",
            after_routing.swap_count,
            after_routing.swap_count,
            log.swap_count()
        )
    } else {
        format!("Final circuit after native decomposition and depth scheduling. {delta}")
    }
}

fn metrics_delta(before: &MappingStageMetrics, after: &MappingStageMetrics) -> String {
    format!(
        "gate_count: {} → {} ({}), depth: {} → {} ({}), swap_count: {} → {} ({}), t_count: {} → {} ({}).",
        before.gate_count,
        after.gate_count,
        signed_delta(before.gate_count, after.gate_count),
        before.depth,
        after.depth,
        signed_delta(before.depth, after.depth),
        before.swap_count,
        after.swap_count,
        signed_delta(before.swap_count, after.swap_count),
        before.t_count,
        after.t_count,
        signed_delta(before.t_count, after.t_count),
    )
}

fn signed_delta(before: u64, after: u64) -> String {
    if after >= before {
        format!("+{}", after - before)
    } else {
        format!("-{}", before - after)
    }
}

fn canon_pair(pair: [u64; 2]) -> [u64; 2] {
    if pair[0] <= pair[1] {
        pair
    } else {
        [pair[1], pair[0]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_parts(swaps: bool) -> MappingTraceParts {
        let mut log = MappingLog {
            initial: vec![(0, 0), (1, 1), (2, 2)],
            events: vec![RawMappingEvent::Interaction {
                gate: "cx".to_string(),
                logical: [0, 2],
                physical: [0, 2],
            }],
            final_layout: vec![(2, 2), (0, 0), (1, 1)],
        };
        let mut after = MappingStageMetrics {
            gate_count: 4,
            depth: 0,
            swap_count: 0,
            t_count: 0,
        };
        let mut final_metrics = MappingStageMetrics {
            gate_count: 4,
            depth: 3,
            swap_count: 0,
            t_count: 0,
        };
        if swaps {
            log.events.insert(
                0,
                RawMappingEvent::Swap {
                    logical: [1, 2],
                    physical: [1, 2],
                },
            );
            log.final_layout = vec![(0, 0), (1, 2), (2, 1)];
            after.gate_count = 5;
            after.swap_count = 1;
            final_metrics.gate_count = 7;
        }
        MappingTraceParts {
            target_id: "line".to_string(),
            edges: vec![(1, 0), (2, 1)],
            log,
            before_routing: MappingStageMetrics {
                gate_count: 4,
                depth: 0,
                swap_count: 0,
                t_count: 0,
            },
            after_routing: after,
            final_metrics,
        }
    }

    #[test]
    fn assemble_narrates_swap_mismatch_and_round_trips() {
        let trace = assemble_mapping_trace(sample_parts(true));
        assert_eq!(trace.schema_version, MAPPING_TRACE_VERSION);
        assert_eq!(trace.kind, MAPPING_TRACE_KIND);
        assert!(trace.summary.contains("1 SWAP"));
        assert!(trace.summary.contains("(1, 2)"));
        assert_eq!(trace.stages.len(), 3);
        assert_eq!(trace.stages[0].id, STAGE_LAYOUT);
        assert_eq!(trace.stages[1].id, STAGE_ROUTING);
        assert_eq!(trace.stages[2].id, STAGE_NATIVE_DECOMP);
        assert!(trace.stages[2].summary.contains("CX triples"));
        assert!(trace.stages[1].summary.contains("swap_count: 0 → 1 (+1)"));
        assert_eq!(trace.topology.edges, vec![[0, 1], [1, 2]]);
        assert_eq!(trace.final_layout[0].logical, 0);
        assert_eq!(trace.final_layout[2].logical, 2);
        let json = trace.to_json_string_pretty().expect("json");
        let back: MappingTrace = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, trace);
    }

    #[test]
    fn assemble_zero_swaps_says_so() {
        let trace = assemble_mapping_trace(sample_parts(false));
        assert!(trace.summary.contains("no SWAPs"));
        assert!(trace.stages[2].summary.contains("Final circuit"));
        assert!(!trace.stages[2].summary.contains("CX triples"));
    }

    #[test]
    fn branch_arms_are_alternatives_not_one_sequence() {
        let mut log = MappingLog::default();
        let shared = vec![(0, 0), (2, 2)];
        log.note_branch(
            BranchArm::Then,
            vec![RawMappingEvent::Swap {
                logical: [0, 2],
                physical: [0, 1],
            }],
            shared.clone(),
        );
        log.note_branch(BranchArm::Else, Vec::new(), shared);
        let trace = assemble_mapping_trace(MappingTraceParts {
            target_id: "line".to_string(),
            edges: vec![(0, 1)],
            log,
            before_routing: MappingStageMetrics::default(),
            after_routing: MappingStageMetrics::default(),
            final_metrics: MappingStageMetrics::default(),
        });
        assert!(trace.summary.contains("exactly one arm runs"));
        assert!(!trace.summary.contains("different permutations"));
        assert!(!trace.summary.contains("inserted 1 SWAP"));
        assert_eq!(trace.events.len(), 2);
        match &trace.events[0] {
            MappingEvent::Branch {
                arm,
                summary,
                events,
                layout,
            } => {
                assert_eq!(*arm, BranchArm::Then);
                assert!(summary.contains("does not also run"));
                assert_eq!(events.len(), 1);
                assert_eq!(layout.len(), 2);
                assert_eq!(layout[0].physical, 0);
            }
            other => panic!("expected then branch, got {other:?}"),
        }
        match &trace.events[1] {
            MappingEvent::Branch { arm, events, .. } => {
                assert_eq!(*arm, BranchArm::Else);
                assert!(events.is_empty());
            }
            other => panic!("expected else branch, got {other:?}"),
        }
    }

    #[test]
    fn disagreed_branch_layouts_are_on_the_event() {
        let mut log = MappingLog::default();
        log.note_branch(
            BranchArm::Then,
            vec![RawMappingEvent::Swap {
                logical: [0, 2],
                physical: [0, 1],
            }],
            vec![(0, 1), (2, 0)],
        );
        log.note_branch(BranchArm::Else, Vec::new(), vec![(0, 0), (2, 2)]);
        let trace = assemble_mapping_trace(MappingTraceParts {
            target_id: "line".to_string(),
            edges: vec![(0, 1)],
            log,
            before_routing: MappingStageMetrics::default(),
            after_routing: MappingStageMetrics::default(),
            final_metrics: MappingStageMetrics::default(),
        });
        assert!(trace.summary.contains("exactly one arm runs"));
        assert!(trace.summary.contains("no single post-branch layout"));
        assert!(trace.summary.contains("pre-branch map"));
        match &trace.events[0] {
            MappingEvent::Branch { layout, .. } => {
                assert_eq!(layout[0].logical, 0);
                assert_eq!(layout[0].physical, 1);
            }
            other => panic!("expected then branch, got {other:?}"),
        }
        match &trace.events[1] {
            MappingEvent::Branch { layout, .. } => {
                assert_eq!(layout[1].physical, 2);
            }
            other => panic!("expected else branch, got {other:?}"),
        }
        let json = trace.to_json_string_pretty().expect("json");
        let back: MappingTrace = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, trace);
    }

    #[test]
    fn mapping_trace_rejects_unknown_field() {
        let trace = assemble_mapping_trace(sample_parts(false));
        let mut value = serde_json::to_value(&trace).expect("value");
        value
            .as_object_mut()
            .expect("object")
            .insert("extra".to_string(), serde_json::json!(1));
        let error = serde_json::from_value::<MappingTrace>(value).expect_err("deny");
        assert!(error.to_string().contains("unknown field"));
    }
}
