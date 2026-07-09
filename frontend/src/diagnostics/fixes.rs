//! Quick-fix generation helpers for structured diagnostics.

use crate::ast::CliffordClass;
use crate::diagnostics::{QuickFix, QuickFixKind, TextEdit};
use crate::lexer::SimpleSpan;
use crate::typecheck::TypeError;

/// Leading whitespace on the line containing `byte_offset`.
fn line_indent(source: &str, byte_offset: usize) -> String {
    let start = source[..byte_offset.min(source.len())]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    source[start..byte_offset.min(source.len())]
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .collect()
}

fn clamp_span(source: &str, span: SimpleSpan) -> SimpleSpan {
    let end = span.end.min(source.len());
    let start = span.start.min(end);
    (start..end).into()
}

/// Apply quick-fix edits to `source` (for tests).
pub fn apply_fixes(source: &str, fixes: &[QuickFix]) -> String {
    let mut out = source.to_owned();
    let mut edits: Vec<_> = fixes.iter().flat_map(|f| f.edits.iter()).collect();
    edits.sort_by_key(|e| std::cmp::Reverse(e.span.start));
    for edit in edits {
        let span = clamp_span(&out, edit.span);
        out.replace_range(span.start..span.end, &edit.replacement);
    }
    out
}

pub fn quick_fixes_for_type_error(err: &TypeError, source: &str) -> Vec<QuickFix> {
    match err {
        TypeError::LinearUnconsumed { name, span, .. } => {
            linear_unconsumed_borrow_fixes(source, name, *span)
        }
        TypeError::CliffordMismatch {
            expected,
            found,
            span,
        } => clifford_mismatch_fixes(source, expected, found, *span),
        TypeError::DepthMismatch {
            expected,
            found,
            span,
        } => depth_mismatch_fixes(source, expected, found, *span),
        TypeError::LinearDiscard {
            bound_name,
            binding_span,
            ..
        } => linear_discard_fixes(source, bound_name.as_deref(), *binding_span),
        _ => Vec::new(),
    }
}

fn linear_unconsumed_borrow_fixes(
    source: &str,
    name: &str,
    _binding_span: SimpleSpan,
) -> Vec<QuickFix> {
    let borrow_header = format!("borrow {name}:");
    if !source.contains(&borrow_header) {
        return Vec::new();
    }

    let discard_pat = format!("discard({name})");
    let reset_pat = format!("reset({name})");
    if source.contains(&discard_pat) || source.contains(&reset_pat) {
        return Vec::new();
    }

    let Some(return_span) = find_return_in_borrow_block(source, name) else {
        return Vec::new();
    };

    let indent = line_indent(source, return_span.start);
    let insert = format!("{indent}{discard_pat}\n");
    vec![
        QuickFix {
            title: format!("Insert discard({name}) before return"),
            kind: QuickFixKind::QuickFix,
            edits: vec![TextEdit {
                span: (return_span.start..return_span.start).into(),
                replacement: insert.clone(),
            }],
            preferred: false,
        },
        QuickFix {
            title: format!("Insert reset({name}) before return"),
            kind: QuickFixKind::QuickFix,
            edits: vec![TextEdit {
                span: (return_span.start..return_span.start).into(),
                replacement: format!("{indent}{reset_pat}\n"),
            }],
            preferred: false,
        },
    ]
}

fn find_return_in_borrow_block(source: &str, name: &str) -> Option<SimpleSpan> {
    let header = format!("borrow {name}:");
    let header_start = source.find(&header)?;
    let after_header = &source[header_start..];
    let brace_rel = after_header.find('{')?;
    let block_start = header_start + brace_rel;

    let block_end = matching_brace(source, block_start)?;
    let block = &source[block_start + 1..block_end];

    let return_rel = block.find("return")?;
    let return_start = block_start + 1 + return_rel;
    Some((return_start..return_start + "return".len()).into())
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn clifford_mismatch_fixes(
    source: &str,
    expected: &CliffordClass,
    found: &CliffordClass,
    span: SimpleSpan,
) -> Vec<QuickFix> {
    if !matches!(found, CliffordClass::Universal) || !matches!(expected, CliffordClass::Clifford) {
        return Vec::new();
    }

    let Some(clifford_span) = find_clifford_token(source, span) else {
        return Vec::new();
    };

    vec![QuickFix {
        title: "Change annotation to Universal".into(),
        kind: QuickFixKind::RefactorRewrite,
        edits: vec![TextEdit {
            span: clifford_span,
            replacement: "Universal".into(),
        }],
        preferred: true,
    }]
}

fn find_clifford_token(source: &str, near: SimpleSpan) -> Option<SimpleSpan> {
    let search_lo = near.start.saturating_sub(256);
    let search_hi = (near.end + 256).min(source.len());
    let region = &source[search_lo..search_hi];
    if let Some(rel) = region.find("Clifford") {
        let start = search_lo + rel;
        return Some((start..start + "Clifford".len()).into());
    }
    None
}

fn depth_mismatch_fixes(
    source: &str,
    expected: &str,
    found: &str,
    _span: SimpleSpan,
) -> Vec<QuickFix> {
    let Some(found_const) = constant_depth_text(found) else {
        return Vec::new();
    };

    let Some(edit_span) = find_depth_field_span(source, expected) else {
        return Vec::new();
    };

    vec![QuickFix {
        title: format!("Update depth annotation to {found_const}"),
        kind: QuickFixKind::RefactorRewrite,
        edits: vec![TextEdit {
            span: edit_span,
            replacement: found_const,
        }],
        preferred: true,
    }]
}

/// When the inferred depth is a concrete natural, return its decimal form.
fn constant_depth_text(found: &str) -> Option<String> {
    let found = found.trim();
    if found.chars().all(|c| c.is_ascii_digit()) {
        return Some(found.to_owned());
    }
    if let Some(inner) = found.strip_prefix("(+ ").and_then(|s| s.strip_suffix(')')) {
        let mut parts = inner.split_whitespace();
        let a: u64 = parts.next()?.parse().ok()?;
        let b: u64 = parts.next()?.parse().ok()?;
        if parts.next().is_none() {
            return Some((a + b).to_string());
        }
    }
    None
}

fn find_depth_field_span(source: &str, token: &str) -> Option<SimpleSpan> {
    let circuit = source.find("Circuit<")?;
    let rest = &source[circuit..];
    for needle in [format!(", {token},"), format!(", {token}, ")] {
        if let Some(rel) = rest.find(&needle) {
            let start = circuit + rel + 2;
            return Some((start..start + token.len()).into());
        }
    }
    None
}

fn linear_discard_fixes(
    source: &str,
    bound_name: Option<&str>,
    binding_span: Option<SimpleSpan>,
) -> Vec<QuickFix> {
    let Some(bound_name) = bound_name else {
        return Vec::new();
    };
    let pattern = format!("let _ = {bound_name}");
    let start = if let Some(span) = binding_span {
        let lo = span.start.saturating_sub(64);
        let hi = (span.end + 16).min(source.len());
        source[lo..hi].find(&pattern).map(|rel| lo + rel)
    } else {
        source.find(&pattern)
    };
    let Some(start) = start else {
        return Vec::new();
    };
    let edit_end = start + pattern.len();

    // Only offer a fix for statement `let _ = x` inside `run { }`, not `let _ = x in e`.
    let after = source[edit_end..].trim_start();
    if after.starts_with("in") {
        return Vec::new();
    }
    let prefix = &source[..start];
    if !prefix.contains("run {") && !prefix.contains("run{") {
        return Vec::new();
    }

    vec![QuickFix {
        title: format!("Replace _ with discard({bound_name})"),
        kind: QuickFixKind::QuickFix,
        edits: vec![TextEdit {
            span: (start..edit_end).into(),
            replacement: format!("discard({bound_name})"),
        }],
        preferred: false,
    }]
}
