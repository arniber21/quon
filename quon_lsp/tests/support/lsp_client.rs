//! JSON-RPC framing client for integration tests.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

pub struct LspMessage {
    pub method: String,
    pub params: Value,
}

pub struct LspClient {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    responses: Receiver<Value>,
    notifications: Receiver<LspMessage>,
    next_id: i64,
    pending_response_id: Option<i64>,
    graceful_shutdown: bool,
}

impl LspClient {
    pub fn spawn() -> Self {
        Self::spawn_with_env(&[])
    }

    pub fn spawn_with_env(extra_env: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_quon_lsp"));
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in extra_env {
            cmd.env(key, value);
        }
        let mut child = cmd.spawn().expect("spawn quon_lsp");
        let stdout = child.stdout.take().expect("stdout");
        let stdin = child.stdin.take().expect("stdin");

        let (resp_tx, resp_rx) = mpsc::channel();
        let (notif_tx, notif_rx) = mpsc::channel();

        thread::spawn(move || read_loop(stdout, resp_tx, notif_tx));

        Self {
            child,
            stdin: Some(stdin),
            responses: resp_rx,
            notifications: notif_rx,
            next_id: 1,
            pending_response_id: None,
            graceful_shutdown: false,
        }
    }

    pub fn send_request(&mut self, method: &str, params: Option<Value>) {
        let id = self.next_id;
        self.next_id += 1;
        let mut msg = serde_json::Map::new();
        msg.insert("jsonrpc".into(), json!("2.0"));
        msg.insert("id".into(), json!(id));
        msg.insert("method".into(), json!(method));
        if let Some(params) = params {
            msg.insert("params".into(), params);
        }
        write_message(&mut self.stdin, &Value::Object(msg));
        self.pending_response_id = Some(id);
    }

    pub fn recv_response(&mut self) -> Value {
        let id = self.pending_response_id.expect("no pending request");
        self.pending_response_id = None;
        self.wait_response(id)
    }

    pub fn send_request_with_response(&mut self, method: &str, params: Option<Value>) -> Value {
        self.send_request(method, params);
        self.recv_response()
    }

    pub fn send_notification(&mut self, method: &str, params: Value) {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        write_message(&mut self.stdin, &msg);
    }

    pub fn wait_notification(&self, method: &str, timeout: Duration) -> Option<LspMessage> {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Ok(msg) = self.notifications.recv_timeout(Duration::from_millis(50)) {
                if msg.method == method {
                    return Some(msg);
                }
            }
        }
        None
    }

    pub fn wait_publish_diagnostics(&self, uri: &str, timeout: Duration) -> Option<Value> {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Ok(msg) = self.notifications.recv_timeout(Duration::from_millis(50)) {
                if msg.method == "textDocument/publishDiagnostics" {
                    let doc_uri = msg.params["uri"].as_str().unwrap_or("");
                    if doc_uri == uri {
                        return Some(msg.params);
                    }
                }
            }
        }
        None
    }

    pub fn shutdown_and_exit(mut self) {
        self.send_request("shutdown", None);
        let _ = self.recv_response();
        self.send_notification("exit", json!({}));
        self.stdin = None;
        self.graceful_shutdown = true;
        let _ = self.child.wait();
    }

    fn wait_response(&self, id: i64) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok(msg) = self.responses.recv_timeout(Duration::from_millis(50)) {
                if msg.get("id") == Some(&json!(id)) {
                    if let Some(err) = msg.get("error") {
                        panic!("LSP error for id {id}: {err}");
                    }
                    return msg.get("result").cloned().unwrap_or(Value::Null);
                }
            }
        }
        panic!("timed out waiting for response id {id}");
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if !self.graceful_shutdown {
            let _ = self.child.kill();
        }
    }
}

fn write_message(stdin: &mut Option<std::process::ChildStdin>, msg: &Value) {
    let body = serde_json::to_string(msg).expect("serialize");
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let stdin = stdin.as_mut().expect("stdin closed");
    stdin.write_all(header.as_bytes()).expect("write header");
    stdin.write_all(body.as_bytes()).expect("write body");
    stdin.flush().expect("flush");
}

fn read_loop(
    stdout: impl Read + Send + 'static,
    responses: Sender<Value>,
    notifications: Sender<LspMessage>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut content_length = None;
        loop {
            let mut line = String::new();
            if reader
                .read_line(&mut line)
                .ok()
                .filter(|&n| n > 0)
                .is_none()
            {
                return;
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                break;
            }
            if let Some(len) = line.strip_prefix("Content-Length: ") {
                content_length = len.parse().ok();
            }
        }
        let Some(len) = content_length else {
            continue;
        };
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&body) else {
            continue;
        };
        if value.get("id").is_some() {
            let _ = responses.send(value);
        } else if let Some(method) = value.get("method").and_then(Value::as_str) {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let _ = notifications.send(LspMessage {
                method: method.to_owned(),
                params,
            });
        }
    }
}
