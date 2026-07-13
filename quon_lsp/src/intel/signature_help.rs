use frontend::analysis::{DocumentAnalysis, signature_site_at};
use tower_lsp::lsp_types::{
    Documentation, MarkupContent, MarkupKind, ParameterInformation, ParameterLabel, Position,
    SignatureHelp, SignatureInformation,
};

use crate::convert::position_to_offset;

pub fn signature_help_at(analysis: &DocumentAnalysis, position: Position) -> Option<SignatureHelp> {
    let offset = position_to_offset(&analysis.src, position)?;
    let site = signature_site_at(analysis, offset)?;
    let parameters = site
        .parameters
        .into_iter()
        .map(|p| ParameterInformation {
            label: ParameterLabel::Simple(p.label),
            documentation: p.documentation.map(|value| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                })
            }),
        })
        .collect();

    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label: site.label,
            documentation: site.documentation.map(|value| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                })
            }),
            parameters: Some(parameters),
            active_parameter: Some(site.active_parameter),
        }],
        active_signature: Some(0),
        active_parameter: Some(site.active_parameter),
    })
}
