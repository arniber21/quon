//! In-file `textDocument/prepareRename` + `textDocument/rename`.
//!
//! # Safety rules
//!
//! Workspace-wide rename is out of scope (single document only).
//!
//! **Allowed targets:** user symbols — function, type alias, type param, parameter,
//! local binding (including linear resources tracked as locals).
//!
//! **Refused:**
//! - Builtins / gates / quantum builtins (no definition we own)
//! - Invalid identifiers (empty, `_`, non-ascii-ident, keywords)
//! - Renames that would break name resolution at any occurrence: if after renaming
//!   `id` → `new_name`, any occurrence site would resolve to a *different* binding,
//!   we refuse. That covers same-scope collisions and intervening shadows — including
//!   cases that would mis-bind linear resources (clone / drop / wrong consumer).
//!
//! Occurrences come from [`frontend::analysis::occurrences_of`] (same as references).

use std::collections::HashMap;

use frontend::analysis::{
    DocumentAnalysis, OccurrenceKind, ResolvedTarget, SymbolId, SymbolKind, keywords,
    occurrences_of, resolve_at,
};
use frontend::lexer::SimpleSpan;
use tower_lsp::jsonrpc::{Error, Result as LspResult};
use tower_lsp::lsp_types::{Position, PrepareRenameResponse, TextEdit, Url, WorkspaceEdit};

use crate::convert::{position_to_offset, span_to_range};

/// Prepare rename at `position`: renameable range + placeholder, or an error reason.
pub fn prepare_rename_at(
    analysis: &DocumentAnalysis,
    position: Position,
) -> LspResult<Option<PrepareRenameResponse>> {
    let Some(offset) = position_to_offset(&analysis.src, position) else {
        return Ok(None);
    };
    let Some(query) = resolve_at(analysis, offset) else {
        return Ok(None);
    };
    match renameable_symbol(analysis, &query.target) {
        Ok(id) => {
            let sym = analysis
                .symbols
                .get(id)
                .ok_or_else(|| Error::invalid_params("symbol missing from index"))?;
            if sym.name_span.start == sym.name_span.end {
                return Err(Error::invalid_params("cannot rename: synthetic symbol"));
            }
            // Prefer the name under the cursor when the user clicked a use site.
            let range = span_to_range(&analysis.src, query.use_span);
            Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                range,
                placeholder: sym.name.clone(),
            }))
        }
        Err(reason) => Err(Error::invalid_params(reason)),
    }
}

/// Rename all in-file occurrences of the symbol under `position` to `new_name`.
pub fn rename_at(
    analysis: &DocumentAnalysis,
    uri: &Url,
    position: Position,
    new_name: &str,
) -> LspResult<Option<WorkspaceEdit>> {
    let Some(offset) = position_to_offset(&analysis.src, position) else {
        return Ok(None);
    };
    let Some(query) = resolve_at(analysis, offset) else {
        return Ok(None);
    };
    let id = match renameable_symbol(analysis, &query.target) {
        Ok(id) => id,
        Err(reason) => return Err(Error::invalid_params(reason)),
    };
    let sym = analysis
        .symbols
        .get(id)
        .ok_or_else(|| Error::invalid_params("symbol missing from index"))?;

    if new_name == sym.name {
        return Ok(Some(WorkspaceEdit {
            changes: Some(HashMap::new()),
            ..Default::default()
        }));
    }

    validate_new_name(new_name)?;

    let occs = occurrences_of(analysis, &query.target);
    if occs.is_empty() {
        return Err(Error::invalid_params("cannot rename: no occurrences found"));
    }

    if let Some(reason) = shadow_conflict(analysis, id, new_name, &occs) {
        return Err(Error::invalid_params(reason));
    }

    let edits: Vec<TextEdit> = occs
        .into_iter()
        .map(|(span, _)| TextEdit {
            range: span_to_range(&analysis.src, span),
            new_text: new_name.to_string(),
        })
        .collect();

    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    }))
}

fn renameable_symbol(
    analysis: &DocumentAnalysis,
    target: &ResolvedTarget,
) -> std::result::Result<SymbolId, &'static str> {
    match target {
        ResolvedTarget::Builtin(_) => Err("cannot rename built-in"),
        ResolvedTarget::Gate(_) => Err("cannot rename gate"),
        ResolvedTarget::QuantumBuiltin(_) => Err("cannot rename quantum built-in"),
        ResolvedTarget::Symbol(id) | ResolvedTarget::TypeAlias(id) => {
            let Some(sym) = analysis.symbols.get(*id) else {
                return Err("cannot rename: unknown symbol");
            };
            match sym.kind {
                SymbolKind::Function
                | SymbolKind::TypeAlias
                | SymbolKind::TypeParam
                | SymbolKind::Parameter
                | SymbolKind::LocalBinding
                | SymbolKind::LinearBinding => Ok(*id),
                SymbolKind::Builtin => Err("cannot rename built-in"),
                SymbolKind::Gate => Err("cannot rename gate"),
                SymbolKind::QuantumBuiltin => Err("cannot rename quantum built-in"),
            }
        }
    }
}

fn validate_new_name(new_name: &str) -> LspResult<()> {
    if !is_valid_ident(new_name) {
        return Err(Error::invalid_params(format!(
            "invalid identifier: `{new_name}`"
        )));
    }
    if keywords().contains(&new_name) {
        return Err(Error::invalid_params(format!("`{new_name}` is a keyword")));
    }
    Ok(())
}

fn is_valid_ident(name: &str) -> bool {
    if name.is_empty() || name == "_" {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn shadow_conflict(
    analysis: &DocumentAnalysis,
    id: SymbolId,
    new_name: &str,
    occs: &[(SimpleSpan, OccurrenceKind)],
) -> Option<String> {
    for (span, _) in occs {
        // Probe at the start of each occurrence; that is where the name is written.
        let resolved = analysis.symbols.resolve_name_at_assuming_rename(
            new_name,
            span.start,
            Some((id, new_name)),
        );
        match resolved {
            Some(found) if found == id => {}
            Some(_) => {
                return Some(format!(
                    "rename to `{new_name}` would shadow or collide with another binding"
                ));
            }
            None => {
                // Our binding should always be visible at its own occurrences.
                return Some(format!(
                    "rename to `{new_name}` would not resolve at an occurrence site"
                ));
            }
        }
    }
    None
}
