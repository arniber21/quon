mod support;

use std::time::Duration;

use serde_json::json;
use support::lsp_client::LspClient;

const TEST_URI: &str = "file:///test.qn";
const INVALID_SRC: &str = "fn f(x: Int): Int = x + y\n";
const VALID_SRC: &str = "fn f(x: Int): Int = x + x\n";

fn init_client(client: &mut LspClient) {
    client.send_request_with_response(
        "initialize",
        Some(json!({
            "processId": std::process::id(),
            "capabilities": {},
            "rootUri": null,
        })),
    );
    client.send_notification("initialized", json!({}));
}

#[test]
fn incremental_diagnostics_update() {
    let mut client = LspClient::spawn_with_env(&[("QUON_LSP_DEBOUNCE_MS", "0")]);
    init_client(&mut client);

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
    assert_eq!(
        params["version"].as_i64(),
        Some(1),
        "didOpen diagnostics should carry version 1"
    );
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(!diags.is_empty(), "expected type error diagnostics");

    let y_start = INVALID_SRC.find('y').expect("y in source") as u32;
    let diag_on_y = diags.iter().find(|d| {
        d["range"]["start"]["character"].as_u64() == Some(u64::from(y_start))
    });
    assert!(
        diag_on_y.is_some(),
        "expected a diagnostic whose start character is on `y` (got {diags:?})"
    );
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
    assert_eq!(
        params["version"].as_i64(),
        Some(2),
        "fixed buffer diagnostics should carry version 2"
    );
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags.is_empty(),
        "expected no errors after fix, got {diags:?}"
    );

    client.shutdown_and_exit();
}

#[test]
fn did_change_full_sync_replaces_buffer() {
    let mut client = LspClient::spawn_with_env(&[("QUON_LSP_DEBOUNCE_MS", "0")]);
    init_client(&mut client);

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
    assert_eq!(params["version"].as_i64(), Some(1));

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
    assert_eq!(
        params["version"].as_i64(),
        Some(2),
        "full-sync diagnostics should carry version 2"
    );
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags.is_empty(),
        "expected clean diagnostics after full replace"
    );

    client.shutdown_and_exit();
}

#[test]
fn did_close_clears_diagnostics() {
    let mut client = LspClient::spawn_with_env(&[("QUON_LSP_DEBOUNCE_MS", "0")]);
    init_client(&mut client);

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
    assert!(!diags.is_empty(), "precondition: error diagnostics present");

    client.send_notification(
        "textDocument/didClose",
        json!({
            "textDocument": { "uri": TEST_URI },
        }),
    );

    let params = client
        .wait_publish_diagnostics(TEST_URI, Duration::from_secs(5))
        .expect("diagnostics after didClose");
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags.is_empty(),
        "didClose should publish empty diagnostics to clear squiggles"
    );

    client.shutdown_and_exit();
}
