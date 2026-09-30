use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::analysis::{AnalysisScheduler, debounce_from_env};
use crate::diagnostics::code_actions_for_range;
use crate::document::{DocumentError, DocumentStore};
use crate::format::format_document;
use crate::intel::{
    completions_at, definition_in_workspace, document_highlight_at, document_symbols,
    folding_ranges, hover_at, inlay_hints, prepare_rename_in_workspace, references_in_workspace,
    rename_in_workspace, semantic_tokens_full, semantic_tokens_legend, semantic_tokens_range,
    signature_help_at,
};
use crate::workspace::{WorkspaceIndex, roots_from_initialize};

struct SaveIndexTask {
    generation: u64,
    handle: tokio::task::JoinHandle<()>,
}

pub struct QuonLanguageServer {
    client: Client,
    documents: Arc<RwLock<DocumentStore>>,
    scheduler: AnalysisScheduler,
    workspace: Arc<RwLock<WorkspaceIndex>>,
    watch_dynamic: AtomicBool,
    /// In-flight `didSave` index tasks. Close aborts them; a finished task
    /// still re-checks the open version before `upsert_open`.
    save_tasks: Arc<Mutex<HashMap<Url, SaveIndexTask>>>,
    save_generation: Arc<AtomicU64>,
}

impl QuonLanguageServer {
    pub fn new(client: Client) -> Self {
        Self::with_debounce(client, debounce_from_env())
    }

    pub fn with_debounce(client: Client, debounce: Duration) -> Self {
        let documents = Arc::new(RwLock::new(DocumentStore::default()));
        let workspace = Arc::new(RwLock::new(WorkspaceIndex::default()));
        let scheduler = AnalysisScheduler::new(
            client.clone(),
            Arc::clone(&documents),
            Arc::clone(&workspace),
            debounce,
        );
        Self {
            client,
            documents,
            scheduler,
            workspace,
            watch_dynamic: AtomicBool::new(false),
            save_tasks: Arc::new(Mutex::new(HashMap::new())),
            save_generation: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Borrow the analysis scheduler. Test-only: lets scheduler cleanup tests
    /// drive `request_analysis`/`cancel_analysis`/`shutdown` and inspect
    /// `pending_count` without spawning the LSP subprocess.
    #[cfg(test)]
    pub fn scheduler(&self) -> &AnalysisScheduler {
        &self.scheduler
    }

    fn scan_workspace(&self) {
        let workspace = Arc::clone(&self.workspace);
        tokio::spawn(async move {
            let workspace_for_scan = Arc::clone(&workspace);
            let scanned = tokio::task::spawn_blocking(move || {
                let roots = {
                    let Ok(index) = workspace_for_scan.read() else {
                        tracing::error!("workspace index read lock poisoned");
                        return None;
                    };
                    index.root_paths()
                };
                Some(WorkspaceIndex::scan_paths(&roots))
            })
            .await;
            let Ok(Some(scanned)) = scanned else {
                return;
            };
            let Ok(mut index) = workspace.write() else {
                tracing::error!("workspace index write lock poisoned");
                return;
            };
            index.replace_disk_units(scanned);
        });
    }

    fn index_text(&self, uri: Url, text: String, version: i32) {
        let workspace = Arc::clone(&self.workspace);
        let documents = Arc::clone(&self.documents);
        let save_tasks = Arc::clone(&self.save_tasks);
        let generation = self.save_generation.fetch_add(1, Ordering::Relaxed);
        let uri_for_task = uri.clone();
        let handle = tokio::spawn(async move {
            let analyzed =
                tokio::task::spawn_blocking(move || frontend::analyze(&text).intelligence).await;
            let Ok(analysis) = analyzed else {
                return;
            };
            // Document lock is held until the index write finishes, so `did_close`
            // cannot `note_closed` between the version check and `upsert_open`.
            let Ok(docs) = documents.read() else {
                tracing::error!("document store read lock poisoned");
                return;
            };
            let Ok(mut index) = workspace.write() else {
                tracing::error!("workspace index write lock poisoned");
                return;
            };
            index.commit_open_analysis(&docs, uri_for_task.clone(), version, analysis);
            drop(index);
            drop(docs);
            if let Ok(mut tasks) = save_tasks.lock() {
                if tasks
                    .get(&uri_for_task)
                    .is_some_and(|task| task.generation == generation)
                {
                    tasks.remove(&uri_for_task);
                }
            }
        });
        match self.save_tasks.lock() {
            Ok(mut tasks) => {
                if let Some(previous) = tasks.insert(uri, SaveIndexTask { generation, handle }) {
                    previous.handle.abort();
                }
            }
            Err(_) => {
                tracing::error!("save-index task mutex poisoned");
                handle.abort();
            }
        }
    }

    fn cancel_save_index(&self, uri: &Url) {
        let Ok(mut tasks) = self.save_tasks.lock() else {
            tracing::error!("save-index task mutex poisoned");
            return;
        };
        if let Some(task) = tasks.remove(uri) {
            task.handle.abort();
        }
    }
}

async fn register_qn_watcher(client: Client) {
    let options = DidChangeWatchedFilesRegistrationOptions {
        watchers: vec![FileSystemWatcher {
            glob_pattern: GlobPattern::String("**/*.qn".into()),
            kind: Some(WatchKind::Create | WatchKind::Change | WatchKind::Delete),
        }],
    };
    let Ok(register_options) = serde_json::to_value(options) else {
        tracing::debug!("could not serialize .qn watcher registration");
        return;
    };
    let registration = Registration {
        id: "quon-workspace-qn".into(),
        method: "workspace/didChangeWatchedFiles".into(),
        register_options: Some(register_options),
    };
    if let Err(err) = client.register_capability(vec![registration]).await {
        tracing::debug!(?err, "client rejected .qn file watcher");
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for QuonLanguageServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Ok(mut index) = self.workspace.write() {
            index.set_roots(roots_from_initialize(&params));
        } else {
            tracing::error!("workspace index write lock poisoned");
        }
        self.watch_dynamic.store(
            params
                .capabilities
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.did_change_watched_files.as_ref())
                .and_then(|watch| watch.dynamic_registration)
                .unwrap_or(false),
            Ordering::Relaxed,
        );
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(true),
                        })),
                        ..Default::default()
                    },
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                document_highlight_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["@".into(), ":".into(), "<".into()]),
                    ..Default::default()
                }),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".into(), ",".into(), "@".into()]),
                    retrigger_characters: Some(vec![",".into()]),
                    ..Default::default()
                }),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: semantic_tokens_legend(),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: Some(true),
                            ..Default::default()
                        },
                    ),
                ),
                // Optional document formatting via embedded quonfmt. Editors that
                // already shell out to quonfmt must not also enable this provider
                // (double-format hazard). quonfmt v1 strips comments — see
                // `crate::format` module docs.
                document_formatting_provider: Some(OneOf::Left(true)),
                code_action_provider: Some(CodeActionProviderCapability::Options(
                    CodeActionOptions {
                        code_action_kinds: Some(vec![
                            CodeActionKind::QUICKFIX,
                            CodeActionKind::REFACTOR_REWRITE,
                        ]),
                        ..Default::default()
                    },
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(false)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.scan_workspace();
        if self.watch_dynamic.load(Ordering::Relaxed) {
            let client = self.client.clone();
            tokio::spawn(async move {
                register_qn_watcher(client).await;
            });
        }
    }

    async fn shutdown(&self) -> Result<()> {
        // Abort outstanding analyses so no analysis work outlives the server.
        self.scheduler.shutdown();
        if let Ok(mut tasks) = self.save_tasks.lock() {
            for (_, task) in tasks.drain() {
                task.handle.abort();
            }
        }
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
        // Cancel any in-flight debounced analysis so it cannot publish stale
        // diagnostics for a document that is no longer open, and reclaim the
        // task handle.
        self.scheduler.cancel_analysis(&uri);
        self.cancel_save_index(&uri);
        if let Ok(mut index) = self.workspace.write() {
            index.note_closed(&uri);
        } else {
            tracing::error!("workspace index write lock poisoned");
        }
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;
        if let Some(text) = params.text {
            let version = {
                let Ok(docs) = self.documents.read() else {
                    tracing::error!("document store read lock poisoned");
                    return;
                };
                match docs.get(&uri) {
                    // Buffer moved on; the scheduler owns the newer text.
                    Some(doc) if doc.text == text => doc.version,
                    Some(_) | None => return,
                }
            };
            self.index_text(uri, text, version);
            return;
        }
        if let Ok(mut index) = self.workspace.write() {
            index.reload_from_disk(&uri);
        } else {
            tracing::error!("workspace index write lock poisoned");
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        for change in params.changes {
            match change.typ {
                FileChangeType::DELETED => {
                    if let Ok(mut index) = self.workspace.write() {
                        if !index.is_open_buffer(&change.uri) {
                            index.remove(&change.uri);
                        }
                    }
                }
                FileChangeType::CREATED | FileChangeType::CHANGED => {
                    let open = self
                        .workspace
                        .read()
                        .map(|index| index.is_open_buffer(&change.uri))
                        .unwrap_or(false);
                    if !open {
                        if let Ok(mut index) = self.workspace.write() {
                            index.reload_from_disk(&change.uri);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
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
        Ok(hover_at(&analysis.intelligence, position))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
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
        let intelligence = analysis.intelligence.clone();
        drop(docs);
        let Ok(index) = self.workspace.read() else {
            tracing::error!("workspace index read lock poisoned");
            return Ok(None);
        };
        Ok(definition_in_workspace(
            &intelligence,
            &uri,
            position,
            &index,
        ))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;
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
        let intelligence = analysis.intelligence.clone();
        drop(docs);
        let Ok(index) = self.workspace.read() else {
            tracing::error!("workspace index read lock poisoned");
            return Ok(None);
        };
        Ok(references_in_workspace(
            &intelligence,
            &uri,
            position,
            include_declaration,
            &index,
        ))
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
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
        Ok(document_highlight_at(&analysis.intelligence, position))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;
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
        let intelligence = analysis.intelligence.clone();
        drop(docs);
        let Ok(index) = self.workspace.read() else {
            tracing::error!("workspace index read lock poisoned");
            return Ok(None);
        };
        prepare_rename_in_workspace(&intelligence, &uri, position, &index)
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;
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
        let intelligence = analysis.intelligence.clone();
        drop(docs);
        let Ok(index) = self.workspace.read() else {
            tracing::error!("workspace index read lock poisoned");
            return Ok(None);
        };
        rename_in_workspace(&intelligence, &uri, position, &new_name, &index)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
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
        Ok(completions_at(&analysis.intelligence, position))
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
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
        Ok(signature_help_at(&analysis.intelligence, position))
    }

    async fn semantic_tokens_full(
        &self,
        _params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = _params.text_document.uri;
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
        Ok(semantic_tokens_full(
            &analysis.intelligence,
            Position {
                line: 0,
                character: 0,
            },
        ))
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
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
        Ok(semantic_tokens_range(&analysis.intelligence, range))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let Ok(docs) = self.documents.read() else {
            tracing::error!("document store read lock poisoned");
            return Ok(None);
        };
        let Some(doc) = docs.get(&uri) else {
            return Ok(None);
        };
        Ok(format_document(&doc.text))
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

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
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
        Ok(document_symbols(&analysis.intelligence))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let uri = params.text_document.uri;
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
        Ok(folding_ranges(&analysis.intelligence))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
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
        Ok(inlay_hints(&analysis.intelligence, range))
    }
}
