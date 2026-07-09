use std::io::{self, Write};
use std::path::Path;

use ariadne::{Color, Label, Report, ReportKind, Source};
use frontend::lexer::SimpleSpan;
use serde::Serialize;
use tower_lsp::lsp_types::{
    Diagnostic as LspDiagnostic, DiagnosticRelatedInformation, DiagnosticSeverity as LspSeverity,
    Location, NumberOrString, Url,
};

use crate::diagnostic::{LintDiagnostic, Severity};

pub mod span {
    use frontend::lexer::SimpleSpan;
    use tower_lsp::lsp_types::{Position, Range};

    /// Byte-offset to LSP line/column (shared with quon_lsp mapping tests).
    pub struct LineIndex {
        line_starts: Vec<usize>,
    }

    impl LineIndex {
        pub fn new(source: &str) -> Self {
            let mut line_starts = vec![0];
            for (i, b) in source.bytes().enumerate() {
                if b == b'\n' {
                    line_starts.push(i + 1);
                }
            }
            Self { line_starts }
        }

        pub fn line_col(&self, offset: usize) -> (u32, u32) {
            let line = self
                .line_starts
                .partition_point(|&start| start <= offset)
                .saturating_sub(1);
            let col = offset.saturating_sub(self.line_starts[line]);
            (line as u32, col as u32)
        }

        pub fn position(&self, offset: usize) -> Position {
            let (line, col) = self.line_col(offset);
            Position {
                line,
                character: col,
            }
        }

        pub fn range(&self, span: SimpleSpan) -> Range {
            Range {
                start: self.position(span.start),
                end: self.position(span.end.min(span.start.saturating_add(1))),
            }
        }
    }
}

use span::LineIndex;

#[derive(Debug, Serialize)]
pub struct JsonOutput {
    pub diagnostics: Vec<LintDiagnostic>,
}

pub fn report_human(path: &Path, source: &str, diags: &[LintDiagnostic]) -> io::Result<()> {
    let file_id = path.display().to_string();
    for diag in diags {
        let span = diag.simple_span();
        let kind = match diag.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
            Severity::Info => ReportKind::Advice,
            Severity::Allow => continue,
        };
        let color = match diag.severity {
            Severity::Error => Color::Red,
            Severity::Warning => Color::Yellow,
            Severity::Info => Color::Cyan,
            Severity::Allow => Color::White,
        };
        let mut report = Report::build(kind, &file_id, span.start)
            .with_message(format!("[{}]: {}", diag.rule, diag.message))
            .with_label(
                Label::new((&file_id, span.start..span.end))
                    .with_message(&diag.message)
                    .with_color(color),
            );
        if let Some(help) = &diag.help {
            report = report.with_help(help);
        }
        report
            .finish()
            .print((&file_id, Source::from(source)))
            .map_err(io::Error::other)?;
    }
    Ok(())
}

pub fn report_github(path: &Path, diags: &[LintDiagnostic]) {
    for diag in diags {
        let level = match diag.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "notice",
            Severity::Allow => continue,
        };
        // GitHub Actions workflow commands use line/col 1-based.
        let line = 1usize;
        let col = diag.span.start + 1;
        eprintln!(
            "::{} file={},line={},col={}::[{}] {}",
            level,
            path.display(),
            line,
            col,
            diag.rule,
            diag.message
        );
    }
}

pub fn diagnostics_to_lsp(source: &str, diags: &[LintDiagnostic], uri: &Url) -> Vec<LspDiagnostic> {
    let line_index = LineIndex::new(source);
    diags
        .iter()
        .map(|d| lint_to_lsp(d, source, &line_index, uri))
        .collect()
}

fn lint_to_lsp(
    diag: &LintDiagnostic,
    source: &str,
    line_index: &LineIndex,
    uri: &Url,
) -> LspDiagnostic {
    let span = clamp_span(source, diag.simple_span());
    let range = line_index.range(span);
    LspDiagnostic {
        range,
        severity: Some(severity_to_lsp(diag.severity)),
        code: Some(NumberOrString::String(diag.rule.clone())),
        code_description: None,
        source: Some("quonlint".into()),
        message: if let Some(help) = &diag.help {
            format!("{} ({help})", diag.message)
        } else {
            diag.message.clone()
        },
        related_information: diag.help.as_ref().map(|help| {
            vec![DiagnosticRelatedInformation {
                location: Location {
                    uri: uri.clone(),
                    range,
                },
                message: help.clone(),
            }]
        }),
        tags: None,
        data: None,
    }
}

fn severity_to_lsp(sev: Severity) -> LspSeverity {
    match sev {
        Severity::Error => LspSeverity::ERROR,
        Severity::Warning => LspSeverity::WARNING,
        Severity::Info => LspSeverity::INFORMATION,
        Severity::Allow => LspSeverity::HINT,
    }
}

fn clamp_span(source: &str, span: SimpleSpan) -> SimpleSpan {
    let end = span.end.min(source.len());
    let start = span.start.min(end);
    (start..end).into()
}

pub fn write_json(out: &mut impl Write, diags: &[LintDiagnostic]) -> io::Result<()> {
    let payload = JsonOutput {
        diagnostics: diags.to_vec(),
    };
    serde_json::to_writer_pretty(&mut *out, &payload)?;
    writeln!(out)
}
