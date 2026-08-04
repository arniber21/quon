use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use frontend;
use quonlint::{LintConfig, diagnostics_to_lsp, lint_source};
use tower_lsp::Client;
use tower_lsp::lsp_types::Url;

use crate::diagnostics::analysis_to_lsp_diags;
use crate::document::DocumentStore;
use crate::span::LineIndex;

/// A pending analysis task paired with the generation it was spawned for.
///
/// The generation lets a completing task distinguish its own map entry from a
/// newer task that superseded it for the same URI, so cleanup never evicts a
/// still-relevant task.
struct PendingTask {
    handle: tokio::task::JoinHandle<()>,
    generation: u64,
}

struct SchedulerState {
    debounce: Duration,
    /// Per-URI debounce task handle; abort on new edit to coalesce.
    pending: HashMap<Url, PendingTask>,
    /// Monotonically increasing tag assigned to each spawned task so successive
    /// tasks for the same URI can be told apart during completion cleanup.
    next_generation: u64,
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
                next_generation: 0,
            })),
            client,
            documents,
        }
    }

    /// Called from LanguageServer `&self` handlers.
    pub fn request_analysis(&self, uri: Url) {
        let client = self.client.clone();
        let documents = Arc::clone(&self.documents);
        let state = Arc::clone(&self.state);

        let Ok(mut guard) = self.state.lock() else {
            tracing::error!("analysis scheduler mutex poisoned");
            return;
        };
        if let Some(task) = guard.pending.remove(&uri) {
            task.handle.abort();
        }
        let debounce = guard.debounce;
        let generation = guard.next_generation;
        guard.next_generation += 1;
        let uri_for_pending = uri.clone();
        let uri_for_cleanup = uri.clone();
        let handle = tokio::spawn(async move {
            // The analysis body lives in its own async block so every early
            // return exits only the block; the cleanup below then always runs
            // and removes our entry while it still represents this task.
            let run = async move {
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
                    let mut diags =
                        analysis_to_lsp_diags(&text_for_task, &result, &line_index, &uri_for_analysis);

                    if result.diagnostics.is_empty() {
                        let lint_path = std::path::Path::new(uri_for_analysis.path());
                        let lint_config = LintConfig::discover_for_file(lint_path);
                        let lints = lint_source(lint_path, &text_for_task, &lint_config);
                        diags.extend(diagnostics_to_lsp(
                            &text_for_task,
                            &lints,
                            &uri_for_analysis,
                        ));
                    }

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
            };
            run.await;

            // Remove our entry only if it still represents this task. A newer
            // request for the same URI would have aborted this task and
            // installed a higher generation, which we must not evict.
            if let Ok(mut guard) = state.lock() {
                if guard.pending.get(&uri_for_cleanup).map(|t| t.generation) == Some(generation) {
                    guard.pending.remove(&uri_for_cleanup);
                }
            }
        });
        guard.pending.insert(
            uri_for_pending,
            PendingTask { handle, generation },
        );
    }

    /// Cancel any pending analysis for `uri` and drop its task handle.
    ///
    /// Called when a document closes so a stale debounced analysis never
    /// publishes diagnostics for a closed document and its handle is reclaimed.
    pub fn cancel_analysis(&self, uri: &Url) {
        let Ok(mut guard) = self.state.lock() else {
            tracing::error!("analysis scheduler mutex poisoned");
            return;
        };
        if let Some(task) = guard.pending.remove(uri) {
            task.handle.abort();
        }
    }

    /// Abort every outstanding analysis task. Called on server shutdown so no
    /// analysis work outlives the server.
    pub fn shutdown(&self) {
        let Ok(mut guard) = self.state.lock() else {
            tracing::error!("analysis scheduler mutex poisoned");
            return;
        };
        for (_, task) in guard.pending.drain() {
            task.handle.abort();
        }
    }

    /// Number of outstanding (not yet cleaned-up) analysis tasks.
    ///
    /// Test/observability helper: a completed task removes its own entry, so a
    /// quiescent scheduler reports zero.
    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.state.lock().map(|g| g.pending.len()).unwrap_or(0)
    }
}

pub fn debounce_from_env() -> Duration {
    std::env::var("QUON_LSP_DEBOUNCE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_millis(100))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::QuonLanguageServer;
    use tower_lsp::LspService;

    /// Drive the real scheduler (and its real `tower_lsp::Client`) in-process.
    /// The server is left uninitialized, so `publish_diagnostics` is suppressed
    /// by tower-lsp (notifications require the `Initialized` state) and never
    /// blocks on the unread `ClientSocket` — letting analysis tasks complete and
    /// run their completion cleanup deterministically.
    fn scheduler(debounce: Duration) -> AnalysisSchedulerView {
        let (service, _socket) =
            LspService::new(move |c| QuonLanguageServer::with_debounce(c, debounce));
        AnalysisSchedulerView {
            _socket,
            service,
        }
    }

    struct AnalysisSchedulerView {
        _socket: tower_lsp::ClientSocket,
        service: tower_lsp::LspService<QuonLanguageServer>,
    }

    impl AnalysisSchedulerView {
        fn sched(&self) -> &AnalysisScheduler {
            self.service.inner().scheduler()
        }
    }

    /// Poll `pending_count` until it equals `target`, or panic on timeout.
    async fn assert_settles(sched: &AnalysisScheduler, target: usize, timeout: Duration) {
        let mut waited = Duration::ZERO;
        let step = Duration::from_millis(2);
        loop {
            let n = sched.pending_count();
            if n == target {
                return;
            }
            if waited >= timeout {
                panic!("pending_count did not settle to {target} within {timeout:?}; still {n}");
            }
            tokio::time::sleep(step).await;
            waited += step;
        }
    }

    fn uri(name: &str) -> Url {
        Url::parse(&format!("file:///{name}.qn")).expect("valid test uri")
    }

    #[tokio::test]
    async fn completed_analysis_removes_pending_entry() {
        let view = scheduler(Duration::from_millis(1));
        let sched = view.sched();
        sched.request_analysis(uri("a"));
        assert_eq!(sched.pending_count(), 1, "task pending immediately after request");
        assert_settles(sched, 0, Duration::from_secs(2)).await;
    }

    #[tokio::test]
    async fn many_unique_documents_all_cleaned_up() {
        let view = scheduler(Duration::from_millis(1));
        let sched = view.sched();
        for i in 0..8 {
            sched.request_analysis(uri(&format!("doc{i}")));
        }
        assert_eq!(sched.pending_count(), 8, "one pending task per unique document");
        assert_settles(sched, 0, Duration::from_secs(2)).await;
    }

    #[tokio::test]
    async fn cancel_analysis_removes_pending_entry() {
        // Long debounce keeps the task in the debounce-sleep phase.
        let view = scheduler(Duration::from_secs(30));
        let sched = view.sched();
        let u = uri("cancel");
        sched.request_analysis(u.clone());
        assert_eq!(sched.pending_count(), 1);
        sched.cancel_analysis(&u);
        assert_eq!(
            sched.pending_count(),
            0,
            "cancel_analysis must remove the pending entry"
        );
    }

    #[tokio::test]
    async fn shutdown_aborts_all_pending_tasks() {
        let view = scheduler(Duration::from_secs(30));
        let sched = view.sched();
        for i in 0..5 {
            sched.request_analysis(uri(&format!("s{i}")));
        }
        assert_eq!(sched.pending_count(), 5);
        sched.shutdown();
        assert_eq!(
            sched.pending_count(),
            0,
            "shutdown must abort and clear every pending task"
        );
    }

    #[tokio::test]
    async fn rapid_requests_coalesce_and_leave_no_stale_handle() {
        let view = scheduler(Duration::from_millis(5));
        let sched = view.sched();
        let u = uri("rapid");
        // Two requests with no await between them: the second aborts the first
        // and installs a single newer entry, so no completed/stale handle lingers.
        sched.request_analysis(u.clone());
        sched.request_analysis(u.clone());
        assert_eq!(
            sched.pending_count(),
            1,
            "rapid requests coalesce to a single pending task"
        );
        assert_settles(sched, 0, Duration::from_secs(2)).await;
    }
}
