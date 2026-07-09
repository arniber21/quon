mod support;

use std::time::Duration;

use serde_json::json;
use support::lsp_client::LspClient;

const TEST_URI: &str = "file:///test.qn";
const INVALID_SRC: &str = "fn f(x: Int): Int = x + y\n";
const VALID_SRC: &str = "fn f(x: Int): Int = x + x\n";

#[test]
fn incremental_diagnostics_update() {
    let mut client = LspClient::spawn_with_env(&[("QUON_LSP_DEBOUNCE_MS", "0")]);

    client.send_request_with_response(
        "initialize",
        Some(json!({
            "processId": std::process::id(),
            "capabilities": {},
            "rootUri": null,
        })),
    );
    client.send_notification("initialized", json!({}));

    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": TEST_URI,
                "languageId": "quon",
                "version": 1,
                "text": INVALID_SRC,
            }
        }),
    );

    let params = client
        .wait_publish_diagnostics(TEST_URI, Duration::from_secs(5))
        .expect("diagnostics after didOpen");
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(!diags.is_empty(), "expected type error diagnostics");

    let y_start = INVALID_SRC.find('y').expect("y in source") as u32;
    let diag_start = diags[0]["range"]["start"]["character"]
        .as_u64()
        .expect("character") as u32;
    assert_eq!(diag_start, y_start, "diagnostic should land on `y`");

    let y_range = json!({
        "start": { "line": 0, "character": y_start },
        "end": { "line": 0, "character": y_start + 1 },
    });
    client.send_notification(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": TEST_URI, "version": 2 },
            "contentChanges": [{
                "range": y_range,
                "text": "x",
            }],
        }),
    );

    let params = client
        .wait_publish_diagnostics(TEST_URI, Duration::from_secs(5))
        .expect("diagnostics after fix");
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags.is_empty(),
        "expected no errors after fix, got {diags:?}"
    );

    client.send_notification(
        "textDocument/didClose",
        json!({
            "textDocument": { "uri": TEST_URI },
        }),
    );

    client.shutdown_and_exit();
}

#[test]
fn did_change_full_sync_replaces_buffer() {
    let mut client = LspClient::spawn_with_env(&[("QUON_LSP_DEBOUNCE_MS", "0")]);
    client.send_request_with_response(
        "initialize",
        Some(json!({ "processId": std::process::id(), "capabilities": {}, "rootUri": null })),
    );
    client.send_notification("initialized", json!({}));

    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": TEST_URI,
                "languageId": "quon",
                "version": 1,
                "text": INVALID_SRC,
            }
        }),
    );
    let _ = client.wait_publish_diagnostics(TEST_URI, Duration::from_secs(5));

    client.send_notification(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": TEST_URI, "version": 2 },
            "contentChanges": [{ "text": VALID_SRC }],
        }),
    );

    let params = client
        .wait_publish_diagnostics(TEST_URI, Duration::from_secs(5))
        .expect("diagnostics after full sync");
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags.is_empty(),
        "expected clean diagnostics after full replace"
    );

    client.shutdown_and_exit();
}
