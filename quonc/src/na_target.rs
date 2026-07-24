//! Map CLI strings onto [`quon_na::pipeline`] NA backend knobs.

use quon_na::PlacerMode;
use quon_na::pipeline::StatePrepMode;

pub use quon_na::NaBackendKind;

pub fn parse_placer_mode(s: &str) -> Result<PlacerMode, String> {
    match s.to_ascii_lowercase().as_str() {
        "routing-agnostic" | "agnostic" | "zac" => Ok(PlacerMode::RoutingAgnostic),
        "routing-aware" | "aware" | "rap" => Ok(PlacerMode::RoutingAware),
        "exact" => Ok(PlacerMode::Exact),
        other => Err(format!(
            "unknown NA placer mode `{other}` (expected routing-agnostic, routing-aware, or exact)"
        )),
    }
}

pub fn parse_na_backend(s: &str) -> Result<NaBackendKind, String> {
    match s.to_ascii_lowercase().as_str() {
        "zoned" | "rap" => Ok(NaBackendKind::Zoned),
        "flat" | "aod" | "enola" => Ok(NaBackendKind::FlatAod),
        other => Err(format!(
            "unknown NA backend `{other}` (expected zoned or flat)"
        )),
    }
}

pub fn parse_state_prep_mode(s: &str) -> Result<StatePrepMode, String> {
    match s.to_ascii_lowercase().as_str() {
        "heuristic" | "greedy" => Ok(StatePrepMode::Heuristic),
        "exact" => Ok(StatePrepMode::Exact),
        other => Err(format!(
            "unknown NA state-prep mode `{other}` (expected heuristic or exact)"
        )),
    }
}
