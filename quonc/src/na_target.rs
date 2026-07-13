//! Map [`backend::NeutralAtomTarget`] descriptors onto `quon_na` planner inputs.

use backend::{AodSpeedModelKind, NeutralAtomTarget, ZoneKind};
use quon_na::{
    CompactionOptions, LegalityLimits, MovementParams, PlacerMode, ZoneSpec, ZonedArchitecture,
};

/// Convert a loaded neutral-atom target into the zoned architecture RAP expects.
pub fn zoned_architecture(na: &NeutralAtomTarget) -> ZonedArchitecture {
    ZonedArchitecture {
        zones: na
            .zones
            .iter()
            .map(|z| ZoneSpec {
                zone_id: z.zone_id,
                kind: z.kind,
                rows: z.rows,
                cols: z.cols,
                origin_um: z.origin_um,
                site_pitch_um: z.site_pitch_um,
                pair_gap_um: z.pair_gap_um,
            })
            .collect(),
        acceleration_m_s2: na.movement.speed_model.acceleration_m_s2,
        trap_transfer_us: na.movement.trap_transfer_us.round() as u64,
        require_readout_zone: false,
    }
}

/// Flat AOD movement parameters derived from the same target JSON.
pub fn movement_params(na: &NeutralAtomTarget) -> MovementParams {
    let pair_gap_um = na
        .zones
        .iter()
        .find(|z| z.kind == ZoneKind::Entanglement)
        .and_then(|z| z.pair_gap_um)
        .unwrap_or(2.0);
    MovementParams {
        acceleration_m_s2: na.movement.speed_model.acceleration_m_s2,
        trap_transfer_us: na.movement.trap_transfer_us.round() as u64,
        rydberg_range_um: na.interaction.rydberg_range_um,
        min_rydberg_spacing_um: na.interaction.min_rydberg_spacing_um,
        min_row_col_separation_um: na.movement.min_row_col_separation_um,
        aod_rows: na.movement.aod_rows,
        aod_cols: na.movement.aod_cols,
        num_aods: na.movement.num_aods,
        pair_gap_um,
        pair_pitch_um: na.interaction.min_rydberg_spacing_um,
        return_home: false,
    }
}

/// Compaction options with optional geometry legality from the target.
pub fn compaction_options(na: &NeutralAtomTarget, greedy: bool) -> CompactionOptions {
    CompactionOptions {
        arch: Some(zoned_architecture(na)),
        legality: Some(LegalityLimits {
            rydberg_range_um: na.interaction.rydberg_range_um,
            min_rydberg_spacing_um: na.interaction.min_rydberg_spacing_um,
            aod_min_separation_um: na.movement.min_row_col_separation_um,
        }),
        greedy,
    }
}

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

/// Which movement/placement backend to run after entangling-layer scheduling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NaBackendKind {
    /// Zoned RAP / ZAC path (#107) — default for multi-zone targets.
    #[default]
    Zoned,
    /// Flat AOD pair-bank planner (#106).
    FlatAod,
}

/// Sanity-check that the target's speed model is the one `quon_na` implements.
pub fn validate_speed_model(na: &NeutralAtomTarget) -> Result<(), String> {
    match na.movement.speed_model.kind {
        AodSpeedModelKind::Sqrt => Ok(()),
    }
}
