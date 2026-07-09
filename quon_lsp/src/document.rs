use std::collections::HashMap;

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
}

#[derive(Debug, Default)]
pub struct DocumentStore {
    pub open: HashMap<Url, Document>,
}

#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("document not open: {0}")]
    NotOpen(Url),
}

impl DocumentStore {
    pub fn open(&mut self, uri: Url, text: String, version: i32) {
        let line_index = LineIndex::new(&text);
        self.open.insert(
            uri.clone(),
            Document {
                uri,
                text,
                version,
                line_index,
            },
        );
    }

    pub fn close(&mut self, uri: &Url) {
        self.open.remove(uri);
    }

    pub fn apply_changes(
        &mut self,
        uri: &Url,
        version: Option<i32>,
        changes: &[TextDocumentContentChangeEvent],
    ) -> Option<&Document> {
        {
            let doc = self.open.get_mut(uri)?;
            if let Some(v) = version {
                doc.version = v;
            }
            for change in changes {
                apply_change(&mut doc.text, change.range, &change.text, &doc.line_index);
                doc.line_index = LineIndex::new(&doc.text);
            }
        }
        self.open.get(uri)
    }
}

fn apply_change(full: &mut String, range: Option<Range>, new_text: &str, line_index: &LineIndex) {
    match range {
        None => {
            *full = new_text.to_owned();
        }
        Some(r) => {
            let start = line_index.offset(r.start);
            let end = line_index.offset(r.end);
            if start <= end && end <= full.len() {
                full.replace_range(start..end, new_text);
            }
        }
    }
}
