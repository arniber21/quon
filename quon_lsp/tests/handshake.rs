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
    assert_eq!(caps["documentSymbolProvider"], true);
    assert_eq!(caps["foldingRangeProvider"], true);
    assert_eq!(caps["inlayHintProvider"], true);
    assert_eq!(
        caps["documentFormattingProvider"], true,
        "expected documentFormattingProvider"
    );
    let sem = &caps["semanticTokensProvider"];
    assert_eq!(
        sem["range"], true,
        "expected semanticTokens range support, got {sem}"
    );
    let mods = sem["legend"]["tokenModifiers"]
        .as_array()
        .expect("tokenModifiers");
    assert!(
        mods.iter().any(|m| m == "definition") && mods.iter().any(|m| m == "readonly"),
        "expected definition+readonly modifiers, got {mods:?}"
    );

    client.send_notification("initialized", json!({}));
    client.shutdown_and_exit();
}
