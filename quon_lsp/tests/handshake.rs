mod support;

use serde_json::json;
use support::lsp_client::LspClient;

#[test]
fn lsp_lifecycle_handshake() {
    let mut client = LspClient::spawn();

    let init = client.send_request_with_response(
        "initialize",
        Some(json!({
            "processId": std::process::id(),
            "capabilities": {},
            "rootUri": null,
        })),
    );
    let sync = &init["capabilities"]["textDocumentSync"];
    assert!(
        sync["change"] == "Incremental" || sync["change"] == 2,
        "expected incremental sync, got {sync}"
    );

    client.send_notification("initialized", json!({}));
    client.shutdown_and_exit();
}
