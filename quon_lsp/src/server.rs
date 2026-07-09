use std::sync::{Arc, RwLock};
use std::time::Duration;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::analysis::{AnalysisScheduler, debounce_from_env};
use crate::diagnostics::code_actions_for_range;
use crate::document::{DocumentError, DocumentStore};

pub struct QuonLanguageServer {
    client: Client,
    documents: Arc<RwLock<DocumentStore>>,
    scheduler: AnalysisScheduler,
}

impl QuonLanguageServer {
    pub fn new(client: Client) -> Self {
        Self::with_debounce(client, debounce_from_env())
    }

    pub fn with_debounce(client: Client, debounce: Duration) -> Self {
        let documents = Arc::new(RwLock::new(DocumentStore::default()));
        let scheduler = AnalysisScheduler::new(client.clone(), Arc::clone(&documents), debounce);
        Self {
            client,
            documents,
            scheduler,
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for QuonLanguageServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        save: None,
                        ..Default::default()
                    },
                )),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::REFACTOR_REWRITE,
                        ]),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {}

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        let version = params.text_document.version;
        if let Ok(mut docs) = self.documents.write() {
            docs.open(uri.clone(), text, version);
        } else {
            tracing::error!("document store write lock poisoned");
            return;
        }
        self.scheduler.request_analysis(uri);
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        let changes = params.content_changes;
        let Ok(mut docs) = self.documents.write() else {
            tracing::error!("document store write lock poisoned");
            return;
        };
        match docs.apply_changes(&uri, Some(version), &changes) {
            Ok(()) => self.scheduler.request_analysis(uri),
            Err(DocumentError::NotOpen(_)) => {
                tracing::debug!(%uri, "did_change for unknown document");
            }
            Err(DocumentError::InvalidEdit(_)) => {
                // Warn already logged in DocumentStore; skip analysis on rejected edit.
            }
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Ok(mut docs) = self.documents.write() {
            docs.close(&uri);
        } else {
            tracing::error!("document store write lock poisoned");
        }
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let range = params.range;
        let Ok(docs) = self.documents.read() else {
            tracing::error!("document store read lock poisoned");
            return Ok(None);
        };
        let Some(doc) = docs.get(&uri) else {
            return Ok(None);
        };
        let Some(analysis) = doc.cached_analysis.as_ref() else {
            return Ok(None);
        };
        let actions = code_actions_for_range(&uri, &doc.text, analysis, range, &doc.line_index);
        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(
                actions
                    .into_iter()
                    .map(CodeActionOrCommand::CodeAction)
                    .collect(),
            ))
        }
    }
}
