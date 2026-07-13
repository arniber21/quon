//! Map CLI strings onto [`quon_na::pipeline`] NA backend knobs.

use quon_na::PlacerMode;

pub use quon_na::NaBackendKind;

pub fn parse_placer_mode(s: &str) -> Result<PlacerMode, String> {
    match s.to_ascii_lowercase().as_str() {
        "routing-agnostic" | "agnostic" | "zac" => Ok(PlacerMode::RoutingAgnostic),
        "routing-aware" | "aware" | "rap" => Ok(PlacerMode::RoutingAware),
        other => Err(format!(
            "unknown NA placer mode `{other}` (expected routing-agnostic or routing-aware)"
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
