use frontend::diagnostics::{AnalysisResult, DiagnosticSeverity, RichDiagnostic};
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, Diagnostic as LspDiagnostic, DiagnosticRelatedInformation,
    DiagnosticSeverity as LspSeverity, NumberOrString, Range, TextEdit, Url, WorkspaceEdit,
};

use crate::span::LineIndex;

pub fn rich_diagnostic_to_lsp(
    diag: &RichDiagnostic,
    source: &str,
    line_index: &LineIndex,
    uri: &Url,
) -> LspDiagnostic {
    let span = clamp_span(source, diag.span);
    let range = span_to_range(source, span, line_index);
    LspDiagnostic {
        range,
        severity: Some(severity_to_lsp(diag.severity)),
        code: Some(NumberOrString::String(diag.code.to_string())),
        code_description: None,
        source: Some("quon".into()),
        message: diag.message.clone(),
        related_information: if diag.related.is_empty() {
            None
        } else {
            Some(
                diag.related
                    .iter()
                    .map(|r| {
                        let rspan = clamp_span(source, r.span);
                        DiagnosticRelatedInformation {
                            location: tower_lsp::lsp_types::Location {
                                uri: uri.clone(),
                                range: span_to_range(source, rspan, line_index),
                            },
                            message: r.message.clone(),
                        }
                    })
                    .collect(),
            )
        },
        tags: None,
        data: None,
    }
}

pub fn analysis_to_lsp_diags(
    src: &str,
    result: &AnalysisResult,
    line_index: &LineIndex,
    uri: &Url,
) -> Vec<LspDiagnostic> {
    result
        .diagnostics
        .iter()
        .map(|d| rich_diagnostic_to_lsp(d, src, line_index, uri))
        .collect()
}

pub fn code_actions_for_range(
    uri: &Url,
    source: &str,
    result: &AnalysisResult,
    range: Range,
    line_index: &LineIndex,
) -> Vec<CodeAction> {
    let mut actions = Vec::new();
    for diag in &result.diagnostics {
        let span = clamp_span(source, diag.span);
        let diag_range = span_to_range(source, span, line_index);
        if !ranges_overlap(&diag_range, &range) {
            continue;
        }
        for fix in &diag.fixes {
            let mut edits = Vec::new();
            for edit in &fix.edits {
                let espan = clamp_span(source, edit.span);
                edits.push(TextEdit {
                    range: span_to_range(source, espan, line_index),
                    new_text: edit.replacement.clone(),
                });
            }
            let kind = match fix.kind {
                frontend::QuickFixKind::QuickFix => CodeActionKind::QUICKFIX,
                frontend::QuickFixKind::RefactorRewrite => CodeActionKind::REFACTOR_REWRITE,
            };
            actions.push(CodeAction {
                title: fix.title.clone(),
                kind: Some(kind),
                diagnostics: None,
                edit: Some(WorkspaceEdit {
                    changes: Some(std::collections::HashMap::from([(uri.clone(), edits)])),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(fix.preferred),
                disabled: None,
                data: None,
            });
        }
    }
    actions
}

fn severity_to_lsp(sev: DiagnosticSeverity) -> LspSeverity {
    match sev {
        DiagnosticSeverity::Error => LspSeverity::ERROR,
        DiagnosticSeverity::Warning => LspSeverity::WARNING,
        DiagnosticSeverity::Info => LspSeverity::INFORMATION,
        DiagnosticSeverity::Hint => LspSeverity::HINT,
    }
}

fn clamp_span(source: &str, span: frontend::lexer::SimpleSpan) -> frontend::lexer::SimpleSpan {
    let end = span.end.min(source.len());
    let start = span.start.min(end);
    (start..end).into()
}

fn span_to_range(
    _source: &str,
    span: frontend::lexer::SimpleSpan,
    line_index: &LineIndex,
) -> Range {
    Range {
        start: line_index.position(span.start),
        end: line_index.position(span.end),
    }
}

fn ranges_overlap(a: &Range, b: &Range) -> bool {
    !(a.end.line < b.start.line
        || (a.end.line == b.start.line && a.end.character <= b.start.character)
        || b.end.line < a.start.line
        || (b.end.line == a.start.line && b.end.character <= a.start.character))
}

// Legacy helpers kept for unit tests that still use plain Diagnostic.
use frontend::diagnostics::Diagnostic;

pub fn diagnostic_to_lsp(diag: &Diagnostic, source: &str, line_index: &LineIndex) -> LspDiagnostic {
    let span = clamp_span(source, diag.span);
    let range = span_to_range(source, span, line_index);
    LspDiagnostic {
        range,
        severity: Some(LspSeverity::ERROR),
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
