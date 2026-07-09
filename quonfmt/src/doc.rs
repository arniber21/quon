//! Wadler-style pretty-printing algebra with width-aware layout.

#![allow(dead_code)]

#[derive(Debug, Clone)]
pub enum Doc {
    Nil,
    Text(String),
    Concat(Vec<Doc>),
    Nest(usize, Box<Doc>),
    Break(BreakKind),
    Group(Box<Doc>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakKind {
    /// Emit a space when flat, newline when broken.
    Soft,
    /// Always emit a newline.
    Hard,
    /// Emit a literal space (never breaks).
    Flat,
}

impl Doc {
    pub fn nil() -> Self {
        Self::Nil
    }

    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    pub fn concat(parts: impl IntoIterator<Item = Doc>) -> Self {
        let parts: Vec<_> = parts.into_iter().collect();
        if parts.is_empty() {
            Self::Nil
        } else if parts.len() == 1 {
            parts.into_iter().next().unwrap_or(Self::Nil)
        } else {
            Self::Concat(parts)
        }
    }

    pub fn nest(indent: usize, doc: Doc) -> Self {
        Self::Nest(indent, Box::new(doc))
    }

    pub fn group(doc: Doc) -> Self {
        Self::Group(Box::new(doc))
    }

    pub fn soft_break() -> Self {
        Self::Break(BreakKind::Soft)
    }

    pub fn hard_break() -> Self {
        Self::Break(BreakKind::Hard)
    }

    pub fn flat_break() -> Self {
        Self::Break(BreakKind::Flat)
    }

    pub fn space() -> Self {
        Self::flat_break()
    }
}

/// Render a document to a string at the given page width.
pub fn render(doc: &Doc, width: usize, indent_unit: &str) -> String {
    let mut out = String::new();
    layout(doc, width, indent_unit, 0, &mut out, Mode::Flat);
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Broken,
}

fn layout(doc: &Doc, width: usize, indent_unit: &str, col: usize, out: &mut String, mode: Mode) {
    match doc {
        Doc::Nil => {}
        Doc::Text(s) => {
            out.push_str(s);
        }
        Doc::Concat(parts) => {
            let mut c = col;
            for part in parts {
                c = layout_with_col(part, width, indent_unit, c, out, mode);
            }
        }
        Doc::Nest(level, inner) => {
            let indent = indent_unit.repeat(*level);
            layout_nested(inner, width, indent_unit, &indent, col, out, mode);
        }
        Doc::Break(kind) => match (kind, mode) {
            (BreakKind::Flat, _) => out.push(' '),
            (BreakKind::Soft, Mode::Flat) => out.push(' '),
            (BreakKind::Soft, Mode::Broken) | (BreakKind::Hard, _) => out.push('\n'),
        },
        Doc::Group(inner) => {
            let mut flat = String::new();
            layout(inner, width, indent_unit, col, &mut flat, Mode::Flat);
            // Flat fit must reject *any* over-width line, not just the final column.
            // Docs with hard newlines (e.g. `match` arms) otherwise look short at the
            // end while an earlier line already exceeded `width`.
            if flat_fits(col, &flat, width) {
                out.push_str(&flat);
            } else {
                layout(inner, width, indent_unit, col, out, Mode::Broken);
            }
        }
    }
}

fn flat_fits(start_col: usize, flat: &str, width: usize) -> bool {
    let mut col = start_col;
    for ch in flat.chars() {
        if ch == '\n' {
            col = 0;
        } else {
            col += 1;
            if col > width {
                return false;
            }
        }
    }
    col <= width
}

fn layout_with_col(
    doc: &Doc,
    width: usize,
    indent_unit: &str,
    col: usize,
    out: &mut String,
    mode: Mode,
) -> usize {
    let start_len = out.len();
    layout(doc, width, indent_unit, col, out, mode);
    // Column after layout is the length of the *last* line, not the byte delta:
    // embedded newlines (e.g. from `match` arms) must reset the column.
    end_column(col, &out[start_len..])
}

fn end_column(start_col: usize, written: &str) -> usize {
    match written.rfind('\n') {
        Some(i) => written[i + 1..].chars().count(),
        None => start_col + written.chars().count(),
    }
}

fn layout_nested(
    doc: &Doc,
    width: usize,
    indent_unit: &str,
    prefix: &str,
    col: usize,
    out: &mut String,
    mode: Mode,
) {
    match doc {
        Doc::Break(BreakKind::Hard) | Doc::Break(BreakKind::Soft) if mode == Mode::Broken => {
            out.push('\n');
            out.push_str(prefix);
        }
        Doc::Concat(parts) => {
            let mut c = col;
            for part in parts {
                if matches!(part, Doc::Break(_)) && mode == Mode::Broken {
                    out.push('\n');
                    out.push_str(prefix);
                    c = prefix.chars().count();
                }
                c = layout_with_col(part, width, indent_unit, c, out, mode);
            }
        }
        _ => layout(doc, width, indent_unit, col, out, mode),
    }
}
