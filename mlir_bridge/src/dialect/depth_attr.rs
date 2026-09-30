//! Shared `depth` attribute check for `quantum.circ` and `quantum.dynamic`.
//!
//! Both dialects store a [`quon_core::DepthExpr`] as a string attribute
//! (ADR-0002). Verifiers must reject a string that is not a depth S-expression,
//! not only a non-string.

use melior::ir::Attribute;
use melior::ir::attribute::StringAttribute;
use quon_core::DepthExpr;

/// Why a `depth` attribute was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DepthAttrError {
    /// The attribute is not a string.
    NotString,
    /// The string is not a [`DepthExpr`] S-expression.
    Malformed,
}

/// Parse a `depth` attribute value as a [`DepthExpr`].
pub(crate) fn parse_depth_attr(value: Attribute<'_>) -> Result<DepthExpr, DepthAttrError> {
    let string = StringAttribute::try_from(value).map_err(|_| DepthAttrError::NotString)?;
    DepthExpr::parse(string.value()).map_err(|_| DepthAttrError::Malformed)
}
