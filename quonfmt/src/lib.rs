//! Canonical Quon source formatter — library API and CLI.
//!
//! See [`docs/quonfmt-style.md`](../docs/quonfmt-style.md) for the normative style spec.

mod config;
mod doc;
mod error;
mod print;

pub use config::StyleConfig;
pub use error::FormatError;

use frontend::ast::Decl;
use frontend::lexer::Sp;

/// Parse and format source; returns formatted string or diagnostics.
pub fn format_str(src: &str) -> Result<String, FormatError> {
    let decls = frontend::parse_program(src).map_err(FormatError::parse)?;
    Ok(format_decls_with(&StyleConfig::default(), &decls))
}

/// Format already-parsed declarations.
pub fn format_decls(decls: &[Sp<Decl>]) -> String {
    format_decls_with(&StyleConfig::default(), decls)
}

/// Format with explicit style configuration (primarily for tests).
pub fn format_decls_with(config: &StyleConfig, decls: &[Sp<Decl>]) -> String {
    let mut ctx = print::Context::new(config);
    let doc = print::decl::print_decls(decls, &mut ctx);
    let rendered = doc::render(&doc, config.max_width, config.indent);
    normalize_output(&rendered)
}

/// Returns `Ok(())` if `src` is already formatted; otherwise an error with a diff hint.
pub fn check_str(src: &str) -> Result<(), FormatError> {
    let formatted = format_str(src)?;
    let normalized_input = normalize_for_compare(src);
    let normalized_output = normalize_for_compare(&formatted);
    if normalized_input == normalized_output {
        Ok(())
    } else {
        Err(FormatError::NotFormatted {
            expected: normalized_output,
        })
    }
}

/// Normalize line endings, strip trailing whitespace, ensure a single final newline.
pub fn normalize_for_compare(s: &str) -> String {
    normalize_output(s)
}

fn normalize_output(s: &str) -> String {
    let mut out = String::new();
    for line in s.replace("\r\n", "\n").lines() {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}
