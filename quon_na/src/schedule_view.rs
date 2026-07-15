//! Debug / visualization schedule JSON envelope (issue #113).
//!
//! `quantum.na` MLIR remains the canonical schedule IR (ADR-0011). This view is
//! the consumer-facing JSON shape for Python tooling and Graphviz-adjacent
//! workflows. `meta.na_placer` / `meta.na_backend` are reserved so a future
//! before/after comparison can key two views without a schema bump.

use serde::{Deserialize, Serialize};

use crate::layout::NeutralAtomLayout;
use crate::pipeline::NaBackendKind;
use crate::report::ResourceReport;
use crate::schedule::ScheduleLayer;
use crate::zoned::{PlacerMode, ZoneKind};

/// Current major version of [`NaScheduleView`].
pub const NA_SCHEDULE_VIEW_VERSION: u32 = 1;

/// Stable wire kind for schedule visualization JSON.
pub const NA_SCHEDULE_VIEW_KIND: &str = "na_schedule_view";

/// Compile-time knobs recorded for tooling / future compare mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NaScheduleViewMeta {
    pub target_id: String,
    pub na_backend: NaBackendKind,
    /// Zoned placer mode (meaningful when `na_backend` is zoned).
    pub na_placer: PlacerMode,
}

/// Zone geometry subset needed to draw extents without the full target JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleViewZone {
    pub zone_id: u32,
    pub kind: ZoneKind,
    pub origin_um: [f64; 2],
    pub width_um: f64,
    pub height_um: f64,
    pub rows: u32,
    pub cols: u32,
    pub site_pitch_um: [f64; 2],
}

/// Debug/visualization schedule document emitted by `--emit-na-schedule`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NaScheduleView {
    pub schema_version: u32,
    pub kind: String,
    pub meta: NaScheduleViewMeta,
    pub metrics: ResourceReport,
    pub zones: Vec<ScheduleViewZone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<NeutralAtomLayout>,
    pub layers: Vec<ScheduleLayer>,
}

impl NaScheduleView {
    /// Build a v1 visualization envelope.
    pub fn new(
        meta: NaScheduleViewMeta,
        metrics: ResourceReport,
        zones: Vec<ScheduleViewZone>,
        layout: Option<NeutralAtomLayout>,
        layers: Vec<ScheduleLayer>,
    ) -> Self {
        Self {
            schema_version: NA_SCHEDULE_VIEW_VERSION,
            kind: NA_SCHEDULE_VIEW_KIND.to_string(),
            meta,
            metrics,
            zones,
            layout,
            layers,
        }
    }

    /// Pretty-printed JSON for CLI emit.
    pub fn to_json_string_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{AtomBinding, AtomId, AtomSite, Position, SiteId, TrapBinding};
    use crate::schedule::{NeutralAtomAction, ScheduleLayer};

    #[test]
    fn schedule_view_round_trips() {
        let view = NaScheduleView::new(
            NaScheduleViewMeta {
                target_id: "generic_rna_v0".into(),
                na_backend: NaBackendKind::Zoned,
                na_placer: PlacerMode::RoutingAgnostic,
            },
            ResourceReport::default(),
            vec![ScheduleViewZone {
                zone_id: 0,
                kind: ZoneKind::Storage,
                origin_um: [0.0, 0.0],
                width_um: 40.0,
                height_um: 20.0,
                rows: 5,
                cols: 10,
                site_pitch_um: [4.0, 4.0],
            }],
            Some(NeutralAtomLayout {
                sites: vec![AtomSite {
                    id: SiteId(0),
                    position: Position {
                        x_um: 1.0,
                        y_um: 2.0,
                    },
                }],
                initial_bindings: vec![AtomBinding {
                    atom: AtomId(0),
                    trap: TrapBinding::Slm { site: SiteId(0) },
                }],
            }),
            vec![ScheduleLayer {
                cycle: 0,
                actions: vec![NeutralAtomAction::Wait { duration_us: 1 }],
            }],
        );
        let text = view.to_json_string_pretty().expect("serialize");
        assert!(text.contains("\"kind\": \"na_schedule_view\""));
        assert!(text.contains("\"schema_version\": 1"));
        assert!(text.contains("\"na_placer\": \"routing_agnostic\""));
        let parsed: NaScheduleView = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(parsed, view);
    }
}
