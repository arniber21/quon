use std::collections::HashMap;

use frontend::AnalysisResult;
use thiserror::Error;
use tower_lsp::lsp_types::{Range, TextDocumentContentChangeEvent, Url};

use crate::span::LineIndex;

#[derive(Debug, Clone)]
pub struct Document {
    pub uri: Url,
    pub text: String,
    pub version: i32,
    /// Used only for LSP Position → byte offset during incremental edit application.
    /// Rebuilt after every text mutation. Analysis rebuilds its own LineIndex from snapshot.
    pub line_index: LineIndex,
    /// Cached structured analysis for the current `version` (code actions read this).
    pub cached_analysis: Option<AnalysisResult>,
}

#[derive(Debug, Default)]
pub struct DocumentStore {
    documents: HashMap<Url, Document>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DocumentError {
    #[error("document not open: {0}")]
    NotOpen(Url),
    #[error("invalid incremental edit for {0}")]
    InvalidEdit(Url),
}

impl DocumentStore {
    pub fn get(&self, uri: &Url) -> Option<&Document> {
        self.documents.get(uri)
    }

    /// Cache structured analysis when the document version still matches.
    pub fn store_cached_analysis_if_current(
        &mut self,
        uri: &Url,
        version: i32,
        analysis: AnalysisResult,
    ) -> bool {
        let Some(doc) = self.documents.get_mut(uri) else {
            return false;
        };
        if doc.version != version {
            return false;
        }
        doc.cached_analysis = Some(analysis);
        true
    }

    pub fn open(&mut self, uri: Url, text: String, version: i32) {
        let line_index = LineIndex::new(&text);
        self.documents.insert(
            uri.clone(),
            Document {
                uri,
                text,
                version,
                line_index,
                cached_analysis: None,
            },
        );
    }

    pub fn close(&mut self, uri: &Url) {
        self.documents.remove(uri);
    }

    pub fn apply_changes(
        &mut self,
        uri: &Url,
        version: Option<i32>,
        changes: &[TextDocumentContentChangeEvent],
    ) -> Result<(), DocumentError> {
        let Some(doc) = self.documents.get_mut(uri) else {
            return Err(DocumentError::NotOpen(uri.clone()));
        };

        let mut next_text = doc.text.clone();
        let mut next_line_index = doc.line_index.clone();

        for change in changes {
            if !apply_change(
                &mut next_text,
                change.range,
                change.range_length,
                &change.text,
                &next_line_index,
            ) {
                tracing::warn!(%uri, ?change.range, "rejected incremental edit");
                return Err(DocumentError::InvalidEdit(uri.clone()));
            }
            next_line_index = LineIndex::new(&next_text);
        }

        doc.text = next_text;
        doc.line_index = next_line_index;
        doc.cached_analysis = None;
        if let Some(v) = version {
            doc.version = v;
        }

        Ok(())
    }
}

fn apply_change(
    full: &mut String,
    range: Option<Range>,
    range_length: Option<u32>,
    new_text: &str,
    line_index: &LineIndex,
) -> bool {
    match range {
        None => {
            *full = new_text.to_owned();
            true
        }
        Some(r) => {
            let Some(start) = line_index.offset(r.start) else {
                return false;
            };
            let end = if let Some(len) = range_length {
                start.saturating_add(len as usize)
            } else {
                let Some(end) = line_index.offset(r.end) else {
                    return false;
                };
                end
            };
            if start <= end && end <= full.len() {
                full.replace_range(start..end, new_text);
                true
            } else {
                false
            }
        }
    }
}
