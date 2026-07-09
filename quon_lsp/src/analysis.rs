use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use frontend;
use tower_lsp::Client;
use tower_lsp::lsp_types::Url;

use crate::diagnostics::analysis_to_lsp_diags;
use crate::document::DocumentStore;
use crate::span::LineIndex;

struct SchedulerState {
    debounce: Duration,
    /// Per-URI debounce task handle; abort on new edit to coalesce.
    pending: HashMap<Url, tokio::task::JoinHandle<()>>,
}

pub struct AnalysisScheduler {
    state: Arc<Mutex<SchedulerState>>,
    client: Client,
    documents: Arc<RwLock<DocumentStore>>,
}

impl AnalysisScheduler {
    pub fn new(client: Client, documents: Arc<RwLock<DocumentStore>>, debounce: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(SchedulerState {
                debounce,
                pending: HashMap::new(),
            })),
            client,
            documents,
        }
    }

    /// Called from LanguageServer `&self` handlers.
    pub fn request_analysis(&self, uri: Url) {
        let client = self.client.clone();
        let documents = Arc::clone(&self.documents);

        let Ok(mut guard) = self.state.lock() else {
            tracing::error!("analysis scheduler mutex poisoned");
            return;
        };
        if let Some(handle) = guard.pending.remove(&uri) {
            handle.abort();
        }
        let debounce = guard.debounce;
        let uri_for_pending = uri.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(debounce).await;

            let (text, version) = {
                let Ok(docs) = documents.read() else {
                    tracing::error!("document store read lock poisoned");
                    return;
                };
                let Some(doc) = docs.get(&uri) else {
                    tracing::debug!(%uri, "analysis skipped: document closed");
                    return;
                };
                (doc.text.clone(), doc.version)
            };

            let uri_for_analysis = uri.clone();
            let text_for_task = text.clone();
            let (lsp_diags, analysis) = match tokio::task::spawn_blocking(move || {
                let result = frontend::analyze(&text_for_task);
                let line_index = LineIndex::new(&text_for_task);
                let diags =
                    analysis_to_lsp_diags(&text_for_task, &result, &line_index, &uri_for_analysis);
                (diags, result)
            })
            .await
            {
                Ok(pair) => pair,
                Err(_) => {
                    tracing::debug!(%uri, "analysis task cancelled");
                    return;
                }
            };

            let should_publish = match documents.write() {
                Ok(mut docs) => docs.store_cached_analysis_if_current(&uri, version, analysis),
                Err(_) => {
                    tracing::error!("document store write lock poisoned");
                    false
                }
            };

            if !should_publish {
                tracing::debug!(%uri, version, "discarding stale diagnostics");
                return;
            }

            client
                .publish_diagnostics(uri, lsp_diags, Some(version))
                .await;
        });
        guard.pending.insert(uri_for_pending, handle);
    }
}

pub fn debounce_from_env() -> Duration {
    std::env::var("QUON_LSP_DEBOUNCE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(100))
}
