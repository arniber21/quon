use frontend::diagnostics::Diagnostic;
use tower_lsp::lsp_types::{Diagnostic as LspDiagnostic, DiagnosticSeverity, Range};

use crate::span::LineIndex;

pub fn diagnostic_to_lsp(diag: &Diagnostic, source: &str, line_index: &LineIndex) -> LspDiagnostic {
    let span = diag.span;
    let end = span.end.min(source.len());
    let start = span.start.min(end);
    let start_pos = line_index.position(start);
    let end_pos = line_index.position(end);
    LspDiagnostic {
        range: Range {
            start: start_pos,
            end: end_pos,
        },
        severity: Some(DiagnosticSeverity::ERROR),
        code: None,
        code_description: None,
        source: Some("quon".into()),
        message: diag.message.clone(),
        related_information: None,
        tags: None,
        data: None,
    }
}

pub fn check_to_lsp_diags(
    src: &str,
    result: Result<(), Vec<Diagnostic>>,
    line_index: &LineIndex,
) -> Vec<LspDiagnostic> {
    match result {
        Ok(()) => vec![],
        Err(diags) => diags
            .iter()
            .map(|d| diagnostic_to_lsp(d, src, line_index))
            .collect(),
    }
}
