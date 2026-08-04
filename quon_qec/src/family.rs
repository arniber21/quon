//! Code-family sizing formulas (architecture_model.md §10).
//!
//! Migrated from `quon_na::qec` so workload IR and NA expansion share one source
//! of truth (ADR-0015).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[cfg(feature = "flux")]
use flux_rs::attrs::*;

/// Net code rate `r = numerator / denominator` (checks included).
///
/// For [`CodeFamily::HighRateQldpcLike`], atoms per logical is `ceil(1/r)` =
/// `ceil(denominator / numerator)`. See architecture_model.md §10.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetRate {
    pub numerator: u32,
    pub denominator: u32,
}

/// Error-correcting code family with a closed-form atoms-per-logical formula.
///
/// Formulas: architecture_model.md §10. Source tags `Repetition` / `Surface`
/// map onto [`CodeFamily::RepetitionCodeToy`] / [`CodeFamily::SurfaceCodeLike`]
/// at the QEC lowering boundary (ADR-0014).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum CodeFamily {
    /// Rotated surface code: `N = 2d² − 1`, `d` odd, `d ≥ 3` (§10.1).
    SurfaceCodeLike { distance: u32 },
    /// Bit-flip repetition toy: `N = 2d − 1`, `d ≥ 2` (§10.2).
    RepetitionCodeToy { distance: u32 },
    /// High-rate qLDPC-like: `N = ceil(1/r)` for net rate `r` (§10.3).
    HighRateQldpcLike { net_rate: NetRate },
    /// User `[[n, k, d]]` block: `N = ceil(n/k)` (§10.4).
    AbstractBlockCode { n: u32, k: u32, d: u32 },
}

/// Failures from validating a [`CodeFamily`] or related sizing helpers.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum QecError {
    #[error("surface-code distance must be odd and >= 3, got {distance}")]
    InvalidSurfaceDistance { distance: u32 },
    #[error("repetition-code distance must be >= 2, got {distance}")]
    InvalidRepetitionDistance { distance: u32 },
    #[error("QLDPC net rate numerator must be > 0")]
    ZeroNetRateNumerator,
    #[error("QLDPC net rate denominator must be > 0")]
    ZeroNetRateDenominator,
    #[error("abstract block code requires n > 0")]
    ZeroPhysicalDimension,
    #[error("abstract block code requires k > 0")]
    ZeroLogicalDimension,
    #[error("abstract block code: expected {expected} logical qubits (k), got {actual}")]
    LogicalQubitCountMismatch { expected: u32, actual: usize },
    #[error("code block must contain at least one logical qubit")]
    EmptyLogicalQubits,
    #[error("code-block atom count overflowed")]
    AtomCountOverflow,
}

/// Atoms per logical qubit for `family`, including syndrome/check ancillas
/// where the family formula accounts for them (architecture_model.md §10).
pub fn atoms_per_logical(family: &CodeFamily) -> Result<u32, QecError> {
    match family {
        CodeFamily::SurfaceCodeLike { distance } => surface_n(*distance),
        CodeFamily::RepetitionCodeToy { distance } => repetition_n(*distance),
        CodeFamily::HighRateQldpcLike { net_rate } => {
            if net_rate.numerator == 0 {
                return Err(QecError::ZeroNetRateNumerator);
            }
            if net_rate.denominator == 0 {
                return Err(QecError::ZeroNetRateDenominator);
            }
            ceil_div(net_rate.denominator, net_rate.numerator)
        }
        CodeFamily::AbstractBlockCode { n, k, d: _ } => {
            if *n == 0 {
                return Err(QecError::ZeroPhysicalDimension);
            }
            if *k == 0 {
                return Err(QecError::ZeroLogicalDimension);
            }
            ceil_div(*n, *k)
        }
    }
}

/// `N(d) = 2d² − 1` for odd `d ≥ 3` (architecture_model.md §10.1).
///
/// The refined spec proves the successful (`Ok`) result equals the closed-form
/// `2*d*d − 1`. The body rejects out-of-domain distances first, then rejects
/// any `d > 46340` — the largest odd `d` with `2*d*d <= u32::MAX` (since
/// `2·46341² > u32::MAX`) — before the unchecked multiplication, so the `Ok`
/// path never wraps. Flux discharges the `Ok` postcondition from the
/// primitive-arithmetic refinement (no `trusted` body, no empty `Result<_, _>`
/// spec).
#[cfg_attr(
    feature = "flux",
    spec(fn(d: u32) -> Result<u32{v: v == 2 * d * d - 1}, QecError>)
)]
pub fn surface_n(distance: u32) -> Result<u32, QecError> {
    if distance < 3 || distance.is_multiple_of(2) {
        return Err(QecError::InvalidSurfaceDistance { distance });
    }
    // `2*d*d` overflows `u32` once `d > 46340` (2·46341² > u32::MAX). Reject
    // before the unchecked multiplication so the `Ok` path never wraps.
    if distance > 46340 {
        return Err(QecError::AtomCountOverflow);
    }
    Ok(2 * distance * distance - 1)
}

/// `N(d) = 2d − 1` for `d ≥ 2` (architecture_model.md §10.2).
///
/// The refined spec proves the successful (`Ok`) result equals the closed-form
/// `2*d − 1`. With overflow checking enabled, the body rejects `d < 2` and
/// `d > u32::MAX/2` (where `2*d` would overflow) before the unchecked
/// arithmetic, so Flux proves both the `Ok` postcondition and the absence of
/// overflow/underflow (no `trusted` body, no empty `Result<_, _>` spec).
#[cfg_attr(feature = "flux", opts(check_overflow = "strict"))]
#[cfg_attr(
    feature = "flux",
    spec(fn(d: u32) -> Result<u32{v: v == 2 * d - 1}, QecError>)
)]
pub fn repetition_n(distance: u32) -> Result<u32, QecError> {
    if distance < 2 {
        return Err(QecError::InvalidRepetitionDistance { distance });
    }
    // `2*d` overflows `u32` once `d > u32::MAX / 2`. Reject before the
    // unchecked multiplication so the `Ok` path never wraps.
    if distance > u32::MAX / 2 {
        return Err(QecError::AtomCountOverflow);
    }
    Ok(2 * distance - 1)
}

/// Ceiling division `(numerator + denominator - 1) / denominator`
/// (architecture_model.md §10.3–§10.4).
///
/// The refined spec proves the successful (`Ok`) result is the mathematical
/// ceiling `⌈numerator / denominator⌉ = (numerator + denominator − 1) /
/// denominator` for any nonzero denominator. With overflow checking enabled,
/// the body rejects a zero denominator and any `numerator + denominator` that
/// would overflow `u32` before the unchecked arithmetic, so Flux proves the
/// `Ok` postcondition and the absence of overflow/underflow (no `trusted`
/// body, no empty `Result<_, _>` spec).
#[cfg_attr(feature = "flux", opts(check_overflow = "strict"))]
#[cfg_attr(
    feature = "flux",
    spec(
        fn(numerator: u32, denominator: u32{denominator > 0}) -> Result<
            u32{v: v == (numerator + denominator - 1) / denominator},
            QecError
        >
    )
)]
// The `(numerator + denominator - 1) / denominator` form is deliberate: Flux
// discharges the `Ok` postcondition from the primitive `/` refinement, but has
// no refined spec for `u32::div_ceil`, so the clippy-suggested rewrite would
// break refinement checking. Keep the form and silence the lint.
#[allow(clippy::manual_div_ceil)]
pub fn ceil_div(numerator: u32, denominator: u32) -> Result<u32, QecError> {
    if denominator == 0 {
        return Err(QecError::ZeroLogicalDimension);
    }
    // `numerator + denominator` overflows `u32` once `numerator > u32::MAX -
    // denominator`. Reject before the unchecked addition so the `Ok` path
    // never wraps.
    if numerator > u32::MAX - denominator {
        return Err(QecError::AtomCountOverflow);
    }
    Ok((numerator + denominator - 1) / denominator)
}

/// Negative compile fixture (issue #411): `ceil_div`'s Flux precondition
/// `denominator > 0` is load-bearing. The `#[should_fail]` attribute encodes
/// that flux *must* reject the zero-denominator call. If someone drops the
/// precondition, this function would verify and flux would error on
/// `should_fail`.
#[allow(dead_code)]
#[cfg_attr(feature = "flux", should_fail)]
fn ceil_div_zero_denominator_is_rejected() -> Result<u32, QecError> {
    ceil_div(1, 0)
}

/// Source-language `CodeFamily` tag (`Repetition` / `Surface`).
///
/// Wire/report strings use `"repetition"` / `"surface"` (ADR-0014).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum SourceFamily {
    Repetition,
    Surface,
}

impl SourceFamily {
    /// Wire/report label for this source family.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceFamily::Repetition => "repetition",
            SourceFamily::Surface => "surface",
        }
    }

    /// Parse a wire/report family string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "repetition" => Some(SourceFamily::Repetition),
            "surface" => Some(SourceFamily::Surface),
            _ => None,
        }
    }

    /// Map onto the backend sizing variant at the QEC lowering boundary.
    pub fn to_code_family(self, distance: u32) -> Result<CodeFamily, QecError> {
        match self {
            SourceFamily::Repetition => {
                let _ = repetition_n(distance)?;
                Ok(CodeFamily::RepetitionCodeToy { distance })
            }
            SourceFamily::Surface => {
                let _ = surface_n(distance)?;
                Ok(CodeFamily::SurfaceCodeLike { distance })
            }
        }
    }
}

impl CodeFamily {
    /// Distance when this family is distance-parameterized; `None` for rate/`[[n,k,d]]`.
    pub fn distance(&self) -> Option<u32> {
        match self {
            CodeFamily::SurfaceCodeLike { distance }
            | CodeFamily::RepetitionCodeToy { distance } => Some(*distance),
            CodeFamily::HighRateQldpcLike { .. } | CodeFamily::AbstractBlockCode { .. } => None,
        }
    }

    /// Source-family tag when this is a v1 source-backed family.
    pub fn source_family(&self) -> Option<SourceFamily> {
        match self {
            CodeFamily::RepetitionCodeToy { .. } => Some(SourceFamily::Repetition),
            CodeFamily::SurfaceCodeLike { .. } => Some(SourceFamily::Surface),
            CodeFamily::HighRateQldpcLike { .. } | CodeFamily::AbstractBlockCode { .. } => None,
        }
    }

    /// Stable wire/report label (`repetition_code_toy`, …).
    pub fn as_report_str(&self) -> &'static str {
        match self {
            CodeFamily::SurfaceCodeLike { .. } => "surface_code_like",
            CodeFamily::RepetitionCodeToy { .. } => "repetition_code_toy",
            CodeFamily::HighRateQldpcLike { .. } => "high_rate_qldpc_like",
            CodeFamily::AbstractBlockCode { .. } => "abstract_block_code",
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn surface_code_overhead_pins() {
        assert_eq!(surface_n(3), Ok(17));
        assert_eq!(surface_n(5), Ok(49));
        assert_eq!(
            atoms_per_logical(&CodeFamily::SurfaceCodeLike { distance: 3 }),
            Ok(17)
        );
    }

    #[test]
    fn surface_code_rejects_even_or_small_distance() {
        assert_eq!(
            surface_n(2),
            Err(QecError::InvalidSurfaceDistance { distance: 2 })
        );
        assert_eq!(
            surface_n(4),
            Err(QecError::InvalidSurfaceDistance { distance: 4 })
        );
    }

    #[test]
    fn repetition_code_overhead_pins() {
        assert_eq!(repetition_n(3), Ok(5));
        assert_eq!(
            atoms_per_logical(&CodeFamily::RepetitionCodeToy { distance: 3 }),
            Ok(5)
        );
    }

    #[test]
    fn source_family_maps_to_backend_variants() {
        assert_eq!(
            SourceFamily::Repetition.to_code_family(3),
            Ok(CodeFamily::RepetitionCodeToy { distance: 3 })
        );
        assert_eq!(
            SourceFamily::Surface.to_code_family(3),
            Ok(CodeFamily::SurfaceCodeLike { distance: 3 })
        );
        assert!(SourceFamily::Repetition.to_code_family(1).is_err());
        assert!(SourceFamily::Surface.to_code_family(4).is_err());
    }

    #[test]
    fn code_family_json_uses_family_tag() {
        let family = CodeFamily::SurfaceCodeLike { distance: 3 };
        let value = serde_json::to_value(&family).expect("serialize");
        assert_eq!(
            value,
            json!({
                "family": "surface_code_like",
                "distance": 3,
            })
        );
        let decoded: CodeFamily = serde_json::from_value(value).expect("deserialize");
        assert_eq!(decoded, family);
    }

    #[test]
    fn code_family_rejects_unknown_json_fields() {
        let value = json!({
            "family": "repetition_code_toy",
            "distance": 3,
            "extra": true,
        });
        assert!(serde_json::from_value::<CodeFamily>(value).is_err());
    }

    #[test]
    fn source_family_wire_strings() {
        assert_eq!(SourceFamily::Repetition.as_str(), "repetition");
        assert_eq!(SourceFamily::Surface.as_str(), "surface");
        assert_eq!(
            SourceFamily::parse("repetition"),
            Some(SourceFamily::Repetition)
        );
        assert_eq!(SourceFamily::parse("surface"), Some(SourceFamily::Surface));
        assert_eq!(SourceFamily::parse("repetition_code_toy"), None);
    }
}
