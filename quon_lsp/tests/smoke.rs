//! Fast LSP protocol smoke tests for the CI tooling job.
//!
//! All tests are `#[ignore]` so `cargo test --workspace` skips them; the tooling
//! job runs `cargo test -p quon_lsp --test smoke -- --include-ignored`.

mod support;

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::json;
use support::lsp_client::LspClient;

const BELL_STATE: &str = include_str!("../../frontend/tests/fixtures/bell_state.qn");
const BELL_URI: &str = "file:///bell_state.qn";
const TEST_URI: &str = "file:///test.qn";
const INVALID_SRC: &str = "fn f(x: Int): Int = x + y\n";

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
#[ignore = "tooling job only"]
fn handshake() {
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

#[test]
#[ignore = "tooling job only"]
fn did_open_clean_file() {
    let mut client = LspClient::spawn_with_env(&[("QUON_LSP_DEBOUNCE_MS", "0")]);
    init_client(&mut client);

    client.send_notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": BELL_URI,
                "languageId": "quon",
                "version": 1,
                "text": BELL_STATE,
            }
        }),
    );

    let params = client
        .wait_publish_diagnostics(BELL_URI, Duration::from_secs(10))
        .expect("diagnostics after didOpen");
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags.is_empty(),
        "bell_state.qn should lint/typecheck clean, got {diags:?}"
    );

    client.shutdown_and_exit();
}

#[test]
#[ignore = "tooling job only"]
fn did_change_incremental() {
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
        .wait_publish_diagnostics(TEST_URI, Duration::from_secs(10))
        .expect("diagnostics after didOpen");
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(!diags.is_empty(), "expected type error diagnostics");

    let y_start = INVALID_SRC.find('y').expect("y in source") as u32;
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
        .wait_publish_diagnostics(TEST_URI, Duration::from_secs(10))
        .expect("diagnostics after fix");
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags.is_empty(),
        "expected no errors after incremental fix, got {diags:?}"
    );

    client.shutdown_and_exit();
}

#[test]
#[ignore = "tooling job only"]
fn did_close() {
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
        .wait_publish_diagnostics(TEST_URI, Duration::from_secs(10))
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
        .wait_publish_diagnostics(TEST_URI, Duration::from_secs(10))
        .expect("diagnostics after didClose");
    let diags = params["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diags.is_empty(),
        "didClose should publish empty diagnostics to clear squiggles"
    );

    client.shutdown_and_exit();
}

#[test]
#[ignore = "tooling job only"]
fn malformed_rpc() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_quon_lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn quon_lsp");

    let mut stdin = child.stdin.take().expect("stdin");
    // Not valid JSON-RPC — server should not panic.
    stdin
        .write_all(b"Content-Length: 5\r\n\r\n{not\n")
        .expect("write malformed frame");
    stdin.flush().expect("flush");
    drop(stdin);

    let mut stdout = child.stdout.take().expect("stdout");
    let mut response = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        match stdout.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                response.extend_from_slice(&buf[..n]);
                if response.windows(2).any(|w| w == b"}\r\n") || response.len() > 4096 {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let _ = child.kill();
    let status = child.wait().expect("wait after kill");
    let body = String::from_utf8_lossy(&response);
    assert!(
        body.contains("error") || body.contains("parse") || !status.success(),
        "malformed RPC should yield JSON-RPC error or non-zero exit, got: {body:?} status={status}"
    );
}
