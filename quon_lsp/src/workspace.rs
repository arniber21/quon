//! Directory-scoped workspace index for top-level `fn` and `type` names.
//!
//! Quon has no import syntax. Until a module system exists, every `.qn` file
//! in one directory shares a namespace for those two declaration kinds.
//! Subdirectories are separate namespaces. See `docs/agents/editor-setup.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use frontend::analysis::{
    DocumentAnalysis, OccurrenceKind, ResolvedTarget, Symbol, SymbolId, SymbolIndex, SymbolKind,
    occurrences_of,
};
use frontend::lexer::{SimpleSpan, Token, lex};
use tower_lsp::lsp_types::{Location, Url};

use crate::convert::span_to_range;

const SKIP_DIR: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    ".venv",
    "grammars",
];

/// Skip pathological inputs. Real Quon sources are far smaller.
const MAX_INDEX_BYTES: u64 = 2_000_000;

struct Unit {
    directory: PathBuf,
    analysis: DocumentAnalysis,
    /// Open-buffer text wins over a disk rescan until the document closes.
    from_open_buffer: bool,
}

/// Indexed `.qn` files grouped by parent directory.
#[derive(Default)]
pub struct WorkspaceIndex {
    roots: Vec<PathBuf>,
    units: HashMap<Url, Unit>,
}

impl WorkspaceIndex {
    pub fn set_roots(&mut self, roots: Vec<PathBuf>) {
        self.roots = roots;
    }

    pub fn scan(&self) -> Vec<(Url, DocumentAnalysis)> {
        Self::scan_paths(&self.roots)
    }

    pub fn root_paths(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    pub fn insert(&mut self, uri: Url, analysis: DocumentAnalysis) {
        self.insert_unit(uri, analysis, false);
    }

    /// Record an open editor buffer. A later disk scan will not overwrite it.
    pub fn upsert_open(&mut self, uri: Url, analysis: DocumentAnalysis) {
        self.insert_unit(uri, analysis, true);
    }

    fn insert_unit(&mut self, uri: Url, analysis: DocumentAnalysis, from_open_buffer: bool) {
        let Some(directory) = directory_key(&uri) else {
            return;
        };
        if let Some(existing) = self.units.get(&uri)
            && existing.from_open_buffer
            && !from_open_buffer
        {
            return;
        }
        self.units.insert(
            uri,
            Unit {
                directory,
                analysis,
                from_open_buffer,
            },
        );
    }

    /// Replace disk-backed units. Open buffers are kept.
    pub fn replace_disk_units(&mut self, scanned: Vec<(Url, DocumentAnalysis)>) {
        let open: HashMap<Url, Unit> = self
            .units
            .drain()
            .filter(|(_, unit)| unit.from_open_buffer)
            .collect();
        for (uri, analysis) in scanned {
            if open.contains_key(&uri) {
                continue;
            }
            self.insert_unit(uri, analysis, false);
        }
        self.units.extend(open);
    }

    pub fn note_closed(&mut self, uri: &Url) {
        let Some(unit) = self.units.get(uri) else {
            return;
        };
        if !unit.from_open_buffer {
            return;
        }
        let on_disk = uri.to_file_path().ok().is_some_and(|path| path.is_file());
        if let Some(unit) = self.units.get_mut(uri) {
            unit.from_open_buffer = false;
        }
        if on_disk {
            self.reload_from_disk(uri);
        } else {
            self.units.remove(uri);
        }
    }

    pub fn is_open_buffer(&self, uri: &Url) -> bool {
        self.units
            .get(uri)
            .is_some_and(|unit| unit.from_open_buffer)
    }

    pub fn remove(&mut self, uri: &Url) {
        self.units.remove(uri);
    }

    /// Re-read `uri` from disk unless an open buffer owns it.
    pub fn reload_from_disk(&mut self, uri: &Url) {
        if self.is_open_buffer(uri) {
            return;
        }
        let Ok(path) = uri.to_file_path() else {
            self.units.remove(uri);
            return;
        };
        if !path.is_file() {
            self.units.remove(uri);
            return;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let analysis = frontend::analyze(&text).intelligence;
                self.insert_unit(uri.clone(), analysis, false);
            }
            Err(err) => {
                tracing::debug!(path = %path.display(), %err, "workspace index skip unreadable file");
            }
        }
    }

    /// Analyze every `.qn` file under `roots`.
    pub fn scan_paths(roots: &[PathBuf]) -> Vec<(Url, DocumentAnalysis)> {
        let mut paths = Vec::new();
        for root in roots {
            if root.is_dir() {
                walk_qn_files(root, &mut paths);
            }
        }
        let mut out = Vec::new();
        for path in paths {
            let Ok(uri) = Url::from_file_path(&path) else {
                continue;
            };
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let analysis = frontend::analyze(&text).intelligence;
                    out.push((uri, analysis));
                }
                Err(err) => {
                    tracing::debug!(path = %path.display(), %err, "workspace index skip unreadable file");
                }
            }
        }
        out
    }

    pub fn definition_locations(&self, from: &Url, name: &str) -> Vec<Location> {
        let Some(directory) = self.directory_of(from) else {
            return Vec::new();
        };
        let mut locs = Vec::new();
        for (uri, unit) in &self.units {
            if unit.directory != directory {
                continue;
            }
            for sym in &unit.analysis.symbols.symbols {
                if sym.name == name && is_workspace_export(&unit.analysis.symbols, sym) {
                    locs.push(Location {
                        uri: uri.clone(),
                        range: span_to_range(&unit.analysis.src, sym.name_span),
                    });
                }
            }
        }
        sort_locations(&mut locs);
        locs
    }

    /// Other indexed files in the same directory as `from` (not `from` itself).
    pub fn other_files<'a>(&'a self, from: &Url) -> Vec<(&'a Url, &'a DocumentAnalysis)> {
        let Some(directory) = self.directory_of(from) else {
            return Vec::new();
        };
        let mut files: Vec<_> = self
            .units
            .iter()
            .filter(|(uri, unit)| unit.directory == directory && *uri != from)
            .map(|(uri, unit)| (uri, &unit.analysis))
            .collect();
        files.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        files
    }

    fn directory_of(&self, uri: &Url) -> Option<PathBuf> {
        self.units
            .get(uri)
            .map(|unit| unit.directory.clone())
            .or_else(|| directory_key(uri))
    }
}

pub fn is_workspace_export(symbols: &SymbolIndex, sym: &Symbol) -> bool {
    if sym.name_span.start == sym.name_span.end {
        return false;
    }
    if !matches!(sym.kind, SymbolKind::Function | SymbolKind::TypeAlias) {
        return false;
    }
    symbols
        .scopes
        .get(sym.scope.0 as usize)
        .and_then(|scope| scope.parent)
        .is_none()
}

/// Identifier occurrences of `name` that are not shadowed by a local binding.
///
/// Used for files that do not themselves define `name`. A lex failure falls
/// back to resolved export occurrences only.
pub fn cross_file_occurrences(
    analysis: &DocumentAnalysis,
    name: &str,
) -> Vec<(SimpleSpan, OccurrenceKind)> {
    let Ok(tokens) = lex(&analysis.src) else {
        return export_occurrences(analysis, name);
    };
    let mut out = Vec::new();
    for (token, span) in tokens {
        let Token::Ident(ident) = token else {
            continue;
        };
        if ident != name || span.start == span.end {
            continue;
        }
        if let Some(kind) = classify_ident(analysis, name, span) {
            out.push((span, kind));
        }
    }
    out.sort_by_key(|(span, _)| (span.start, span.end));
    out.dedup_by_key(|(span, _)| (span.start, span.end));
    out
}

fn classify_ident(
    analysis: &DocumentAnalysis,
    name: &str,
    span: SimpleSpan,
) -> Option<OccurrenceKind> {
    if let Some(id) = analysis.symbols.by_def_span(span) {
        let sym = analysis.symbols.get(id)?;
        if sym.name == name && is_workspace_export(&analysis.symbols, sym) {
            return Some(OccurrenceKind::Write);
        }
        return None;
    }
    if let Some(id) = analysis.symbols.resolve_name_at(name, span.start) {
        let sym = analysis.symbols.get(id)?;
        if sym.name == name && is_workspace_export(&analysis.symbols, sym) {
            return Some(if sym.name_span == span {
                OccurrenceKind::Write
            } else {
                OccurrenceKind::Read
            });
        }
        return None;
    }
    Some(OccurrenceKind::Read)
}

fn export_occurrences(
    analysis: &DocumentAnalysis,
    name: &str,
) -> Vec<(SimpleSpan, OccurrenceKind)> {
    let mut out = Vec::new();
    for id in export_ids(analysis, name) {
        let target = export_target(analysis, id);
        out.extend(occurrences_of(analysis, &target));
    }
    out.sort_by_key(|(span, _)| (span.start, span.end));
    out.dedup_by_key(|(span, _)| (span.start, span.end));
    out
}

pub fn export_ids(analysis: &DocumentAnalysis, name: &str) -> Vec<SymbolId> {
    analysis
        .symbols
        .symbols
        .iter()
        .filter(|sym| sym.name == name && is_workspace_export(&analysis.symbols, sym))
        .map(|sym| sym.id)
        .collect()
}

pub fn export_target(analysis: &DocumentAnalysis, id: SymbolId) -> ResolvedTarget {
    match analysis.symbols.get(id).map(|sym| sym.kind) {
        Some(SymbolKind::TypeAlias) => ResolvedTarget::TypeAlias(id),
        _ => ResolvedTarget::Symbol(id),
    }
}

fn directory_key(uri: &Url) -> Option<PathBuf> {
    let path = uri.to_file_path().ok()?;
    let parent = path.parent()?.to_path_buf();
    Some(std::fs::canonicalize(&parent).unwrap_or(parent))
}

fn walk_qn_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::debug!(path = %dir.display(), %err, "workspace index skip directory");
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if let Some(name) = path.file_name().and_then(|name| name.to_str())
                && (SKIP_DIR.contains(&name) || name.starts_with('.'))
            {
                continue;
            }
            walk_qn_files(&path, out);
        } else if meta.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("qn") {
            if meta.len() > MAX_INDEX_BYTES {
                tracing::debug!(path = %path.display(), "workspace index skip large file");
                continue;
            }
            out.push(path);
        }
    }
}

fn sort_locations(locs: &mut [Location]) {
    locs.sort_by(|a, b| {
        a.uri
            .as_str()
            .cmp(b.uri.as_str())
            .then(a.range.start.line.cmp(&b.range.start.line))
            .then(a.range.start.character.cmp(&b.range.start.character))
    });
}

/// Workspace roots from `initialize`, including the pre-folders `root_uri`.
pub fn roots_from_initialize(params: &tower_lsp::lsp_types::InitializeParams) -> Vec<PathBuf> {
    if let Some(folders) = &params.workspace_folders {
        return folders
            .iter()
            .filter_map(|folder| folder.uri.to_file_path().ok())
            .collect();
    }
    params
        .root_uri
        .as_ref()
        .and_then(|uri| uri.to_file_path().ok())
        .into_iter()
        .collect()
}
