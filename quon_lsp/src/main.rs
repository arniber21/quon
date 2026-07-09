use tower_lsp::{LspService, Server};

use quon_lsp::QuonLanguageServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(QuonLanguageServer::new);
    Server::new(stdin, stdout, socket).serve(service).await;
    Ok(())
}
