mod support;

use std::time::Duration;

use serde_json::json;
use support::lsp_client::LspClient;

const TEST_URI: &str = "file:///diagnostics.qn";

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

fn open_doc(client: &mut LspClient, version: i32, text: &str) {
    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": TEST_URI,
                "languageId": "quon",
                "version": version,
                "text": text,
            }
        }),
    );
}

fn wait_diags(client: &LspClient) -> serde_json::Value {
    client
        .wait_publish_diagnostics(TEST_URI, Duration::from_secs(5))
        .expect("publishDiagnostics")
}

#[test]
fn publish_diagnostics_includes_stable_code() {
    let mut client = LspClient::spawn_with_env(&[("QUON_LSP_DEBOUNCE_MS", "0")]);
    init_client(&mut client);
    open_doc(
        &mut client,
        1,
        "fn f(): Q<Int> = run {\n  borrow a: Qubit in {\n    return 0\n  }\n}",
    );
    let params = wait_diags(&client);
    let diags = params["diagnostics"].as_array().expect("array");
    assert!(!diags.is_empty());
    assert_eq!(diags[0]["code"].as_str(), Some("quon.linearity.unconsumed"));
    client.shutdown_and_exit();
}

#[test]
fn code_action_returns_borrow_discard_fix() {
    let src = "fn f(): Q<Int> = run {\n  borrow a: Qubit in {\n    return 0\n  }\n}";
    let mut client = LspClient::spawn_with_env(&[("QUON_LSP_DEBOUNCE_MS", "0")]);
    init_client(&mut client);
    open_doc(&mut client, 1, src);
    let params = wait_diags(&client);
    let range = params["diagnostics"][0]["range"].clone();

    client.send_request(
        "textDocument/codeAction",
        Some(json!({
            "textDocument": { "uri": TEST_URI },
            "range": range,
            "context": { "diagnostics": params["diagnostics"] },
        })),
    );
    let result = client.recv_response();
    let actions = result.as_array().expect("code action array");
    assert!(
        actions.iter().any(|a| {
            a["title"]
                .as_str()
                .is_some_and(|t| t.contains("discard(a)"))
        }),
        "actions: {actions:?}"
    );
    client.shutdown_and_exit();
}

#[test]
fn code_action_clifford_fix_clears_diagnostic() {
    let src = "fn f(): Circuit<1, 1, 1, Clifford> = circuit { T @0 }";
    let mut client = LspClient::spawn_with_env(&[("QUON_LSP_DEBOUNCE_MS", "0")]);
    init_client(&mut client);
    open_doc(&mut client, 1, src);
    let params = wait_diags(&client);
    let range = params["diagnostics"][0]["range"].clone();

    client.send_request(
        "textDocument/codeAction",
        Some(json!({
            "textDocument": { "uri": TEST_URI },
            "range": range,
            "context": { "diagnostics": params["diagnostics"] },
        })),
    );
    let result = client.recv_response();
    let actions = result.as_array().expect("code actions");
    let edit = &actions[0]["edit"]["changes"][TEST_URI][0];
    let new_text = edit["newText"].as_str().unwrap();
    let start = edit["range"]["start"]["character"].as_u64().unwrap() as usize;
    let end = edit["range"]["end"]["character"].as_u64().unwrap() as usize;
    let mut fixed = src.to_owned();
    fixed.replace_range(start..end, new_text);

    client.send_notification(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": TEST_URI, "version": 2 },
            "contentChanges": [{ "text": fixed }],
        }),
    );
    let params = wait_diags(&client);
    let diags = params["diagnostics"].as_array().expect("array");
    assert!(
        diags.is_empty(),
        "expected clean after Clifford fix, got {diags:?}"
    );
    client.shutdown_and_exit();
}

#[test]
fn truncated_source_never_panics_lsp() {
    let bell = include_str!("../../frontend/tests/fixtures/bell_state.qn");
    let mut client = LspClient::spawn_with_env(&[("QUON_LSP_DEBOUNCE_MS", "0")]);
    init_client(&mut client);
    for len in (1..bell.len()).step_by(17) {
        open_doc(&mut client, len as i32, &bell[..len]);
        let _ = client.wait_publish_diagnostics(TEST_URI, Duration::from_millis(500));
    }
    client.shutdown_and_exit();
}
