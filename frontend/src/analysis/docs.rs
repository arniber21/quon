//! Leading documentation comments for symbols (issue #173).
//!
//! Quon has no dedicated doc syntax in v1. Any `--` line comments or `{- -}`
//! block comments that appear immediately above a top-level `fn` / `type`
//! (only whitespace between the comment run and the declaration) become that
//! symbol's documentation. Comments are skipped by the lexer, so we recover
//! them from the original source using the declaration's start offset.

/// Collect leading comments immediately before `decl_start` and return their
/// body text (delimiters stripped), or `None` if there is no such run.
pub fn extract_leading_docs(src: &str, decl_start: usize) -> Option<String> {
    let decl_start = decl_start.min(src.len());
    let mut end = trim_end_whitespace(src, decl_start);
    if end == 0 {
        return None;
    }

    let mut chunks: Vec<String> = Vec::new();
    loop {
        end = trim_end_whitespace(src, end);
        if end == 0 {
            break;
        }
        if let Some((start, body)) = take_trailing_block_comment(src, end) {
            chunks.push(body);
            end = start;
            continue;
        }
        if let Some((start, body)) = take_trailing_line_comment(src, end) {
            chunks.push(body);
            end = start;
            continue;
        }
        break;
    }

    if chunks.is_empty() {
        return None;
    }
    chunks.reverse();
    let text = chunks.join("\n");
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn trim_end_whitespace(src: &str, end: usize) -> usize {
    let bytes = src.as_bytes();
    let mut i = end;
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    i
}

/// If `src[..end]` ends with a complete `{- … -}` (nested), return
/// `(start_of_comment, body)`.
fn take_trailing_block_comment(src: &str, end: usize) -> Option<(usize, String)> {
    let bytes = src.as_bytes();
    if end < 2 || &bytes[end - 2..end] != b"-}" {
        return None;
    }
    let mut depth = 1usize;
    let mut i = end - 2;
    while i > 0 {
        i -= 1;
        if bytes[i] == b'{' && i + 1 < end && bytes[i + 1] == b'-' {
            depth -= 1;
            if depth == 0 {
                let body = src[i + 2..end - 2].trim();
                return Some((i, body.to_string()));
            }
        } else if bytes[i] == b'-' && i + 1 < end && bytes[i + 1] == b'}' {
            depth += 1;
        }
    }
    None
}

/// If `src[..end]` ends on a `-- …` line (no newline after the comment body),
/// return `(start_of_`--`, body)`. Lines with code before `--` are rejected.
fn take_trailing_line_comment(src: &str, end: usize) -> Option<(usize, String)> {
    let line_start = src[..end].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &src[line_start..end];
    let content = line.trim_start();
    if !content.starts_with("--") {
        return None;
    }
    let dash_at = line_start + (line.len() - content.len());
    let body = content[2..].trim();
    Some((dash_at, body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_comment() {
        let src = "-- Makes a Bell pair\nfn bell_state(): Int = 1\n";
        let start = src.find("fn ").expect("fn");
        assert_eq!(
            extract_leading_docs(src, start).as_deref(),
            Some("Makes a Bell pair")
        );
    }

    #[test]
    fn stacked_line_comments() {
        let src = "-- line one\n-- line two\nfn f(): Int = 1\n";
        let start = src.find("fn ").expect("fn");
        assert_eq!(
            extract_leading_docs(src, start).as_deref(),
            Some("line one\nline two")
        );
    }

    #[test]
    fn block_comment() {
        let src = "{- Bell pair prep -}\nfn bell_state(): Int = 1\n";
        let start = src.find("fn ").expect("fn");
        assert_eq!(
            extract_leading_docs(src, start).as_deref(),
            Some("Bell pair prep")
        );
    }

    #[test]
    fn blank_line_between_comment_and_decl_still_attaches() {
        let src = "-- docs\n\nfn f(): Int = 1\n";
        let start = src.find("fn ").expect("fn");
        assert_eq!(extract_leading_docs(src, start).as_deref(), Some("docs"));
    }

    #[test]
    fn code_between_breaks_attachment() {
        let src = "-- orphan\nfn a(): Int = 1\nfn b(): Int = 2\n";
        let start = src.find("fn b").expect("fn b");
        assert_eq!(extract_leading_docs(src, start), None);
    }

    #[test]
    fn preceding_decl_does_not_steal_next_docs() {
        let src = "fn a(): Int = 1\n-- for b\nfn b(): Int = 2\n";
        let start = src.find("fn b").expect("fn b");
        assert_eq!(extract_leading_docs(src, start).as_deref(), Some("for b"));
    }
}
