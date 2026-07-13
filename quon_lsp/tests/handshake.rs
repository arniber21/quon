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
    let caps = &init["capabilities"];
    let sync = &caps["textDocumentSync"];
    assert!(
        sync["change"] == "Incremental" || sync["change"] == 2,
        "expected incremental sync, got {sync}"
    );
    assert_eq!(caps["definitionProvider"], true);
    assert_eq!(caps["referencesProvider"], true);
    assert_eq!(caps["documentHighlightProvider"], true);
    assert_eq!(caps["renameProvider"]["prepareProvider"], true);

    client.send_notification("initialized", json!({}));
    client.shutdown_and_exit();
}
