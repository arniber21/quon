use frontend::analysis::DocumentAnalysis;
use frontend::ast::{Decl, Expr};
use frontend::lexer::{SimpleSpan, Sp};
use frontend::visitor::{Traversal, Visitor};
use tower_lsp::lsp_types::FoldingRange;

use crate::convert::offset_to_position;

/// Folding ranges for `circuit` / `run` / `borrow` / `match` / `for` and multi-line decls.
///
/// Structural recursion is delegated to [`frontend::visitor`] (issue #399); this
/// module only decides, per visited node, whether its span is a useful fold
/// region (pre-order push before descending into children).
pub fn folding_ranges(analysis: &DocumentAnalysis) -> Option<Vec<FoldingRange>> {
    let mut ranges = Vec::new();
    {
        let mut visitor = FoldingRangeVisitor {
            src: &analysis.src,
            out: &mut ranges,
        };
        frontend::visitor::walk_program(&mut visitor, &analysis.decls);
    }
    if ranges.is_empty() {
        None
    } else {
        Some(ranges)
    }
}

struct FoldingRangeVisitor<'a> {
    src: &'a str,
    out: &'a mut Vec<FoldingRange>,
}

impl<'a> Visitor for FoldingRangeVisitor<'a> {
    fn visit_decl_pre(&mut self, decl: &Sp<Decl>) -> Traversal {
        // A multi-line declaration is itself a fold region.
        push_fold(self.out, self.src, decl.1);
        Traversal::Recurse
    }

    fn visit_expr_pre(&mut self, expr: &Sp<Expr>) -> Traversal {
        let span = expr.1;
        match &expr.0 {
            // Block-like constructs always anchor a fold.
            Expr::CircuitBlock(_)
            | Expr::RunBlock(_)
            | Expr::Borrow { .. }
            | Expr::Match { .. }
            | Expr::For { .. } => push_fold(self.out, self.src, span),
            // Desugared `run { … }` keeps the original block span on the Bind.
            Expr::Bind { .. } => {
                if looks_like_keyword(self.src, span, "run") {
                    push_fold(self.out, self.src, span);
                }
            }
            // These only fold when they actually span multiple lines.
            Expr::Let { .. } | Expr::If { .. } | Expr::Lam { .. }
                if span_multiline(self.src, span) =>
            {
                push_fold(self.out, self.src, span);
            }
            _ => {}
        }
        Traversal::Recurse
    }
}

fn looks_like_keyword(src: &str, span: SimpleSpan, kw: &str) -> bool {
    let start = span.start.min(src.len());
    let end = span.end.min(src.len());
    if start >= end {
        return false;
    }
    src[start..end].trim_start().starts_with(kw)
}

fn span_multiline(src: &str, span: SimpleSpan) -> bool {
    let start = offset_to_position(src, span.start);
    let end = offset_to_position(src, span.end);
    end.line > start.line
}

fn push_fold(out: &mut Vec<FoldingRange>, src: &str, span: SimpleSpan) {
    let start = offset_to_position(src, span.start);
    let end = offset_to_position(src, span.end);
    if end.line <= start.line {
        return;
    }
    // Prefer folding interior lines so the header keyword stays visible.
    let end_line = if end.character == 0 && end.line > start.line {
        end.line - 1
    } else {
        end.line
    };
    if end_line <= start.line {
        return;
    }
    out.push(FoldingRange {
        start_line: start.line,
        start_character: Some(start.character),
        end_line,
        end_character: Some(end.character),
        kind: None,
        collapsed_text: None,
    });
}
