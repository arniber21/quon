# Issue #43 — Adversarial Code Review

Review of the LSP foundation implementation (`quon_lsp` crate) on branch `issue-43-lsp-foundation`.

## Summary

Seven findings (2 critical, 5 major) were identified during adversarial review of incremental edit handling, span mapping, integration test coverage, Taskless rule scope, test harness robustness, and `DocumentStore` encapsulation. All were fixed in this pass.

## Findings and Fixes

### C1 — Invalid incremental edits silently dropped

**Finding:** `apply_change` silently skipped edits when LSP ranges were out of bounds or mapped to invalid byte offsets. The server still scheduled analysis on the unchanged buffer, masking client/server desync.

**Fix:**
- `apply_change` now returns `bool`; `apply_changes` returns `Result<(), DocumentError>` with a new `InvalidEdit` variant.
- Rejected edits emit `tracing::warn!` with URI and range.
- `did_change` skips analysis when `apply_changes` returns `InvalidEdit`.
- Document version is only bumped after all edits succeed.
- Unit test `invalid_edit_is_rejected_without_mutation` verifies buffer and version are unchanged on rejection.

### C2 — `LineIndex::offset` defaulted to 0 on invalid positions

**Finding:** `LineIndex::offset` used `.unwrap_or(0)`, turning invalid LSP positions into a silent edit at the start of the file — a data-corruption vector for incremental sync.

**Fix:**
- `offset` now returns `Option<usize>`.
- `apply_change` rejects edits when either endpoint fails to map.
- Round-trip and invalid-position unit tests updated accordingly.

### M1 — Integration tests did not assert diagnostic version

**Finding:** `incremental_lsp.rs` checked diagnostic content but not the `version` field on `textDocument/publishDiagnostics`, leaving stale-version publishing untested.

**Fix:** All integration tests now assert `params["version"]` matches the expected document version after `didOpen`, incremental fix, and full sync.

### M2 — Span assertions incomplete; missing unicode string diagnostic test

**Finding:** `assert_lsp_span_on` only checked range start, not end. The existing unicode-in-string test verified column math but not full diagnostic span mapping through `diagnostic_to_lsp`.

**Fix:**
- `assert_lsp_span_on` now asserts both start and end positions.
- Added `unicode_in_string_literal_diagnostic_span` exercising full span mapping with a UTF-16 string literal prefix.
- Applied `assert_lsp_span_on` consistently across span mapping tests.

### M3 — `didClose` clearing diagnostics untested

**Finding:** `did_close` publishes empty diagnostics to clear editor squiggles, but no integration test verified this behavior.

**Fix:** Added `did_close_clears_diagnostics` integration test that opens an errored document, closes it, and asserts an empty diagnostics notification is received.

### M4 — Taskless ignore too broad for `quon_lsp`

**Finding:** `.taskless/rules/no-anyhow-in-lib-src.yml` ignored the entire `quon_lsp/**` tree, exempting library source (`lib.rs`, `document.rs`, etc.) from the no-anyhow rule. Only `main.rs` legitimately uses `anyhow` for the CLI entrypoint.

**Fix:** Narrowed ignore to `quon_lsp/src/main.rs` only.

### M5 — Test harness stderr pipe could block

**Finding:** `LspClient::spawn_with_env` piped stderr without a reader. Verbose tracing (`QUON_LOG=debug`) could fill the pipe buffer and deadlock the server subprocess during integration tests.

**Fix:** Redirect stderr to `Stdio::null()` in the test harness.

### M6 — `DocumentStore.open` field was public

**Finding:** The internal `HashMap<Url, Document>` was exposed as a public `open` field, allowing callers to bypass `apply_changes` validation and mutate documents directly.

**Fix:**
- Renamed field to private `documents`.
- Added public `get(&self, uri: &Url) -> Option<&Document>`.
- Updated `analysis.rs` and unit tests to use `get()`.

## Validation

```text
cargo fmt --check
cargo clippy -p quon_lsp --all-targets -- -D warnings
cargo test -p quon_lsp
```

## Residual Notes

- `LineIndex::position` still clamps out-of-range byte offsets to line 0 (diagnostic mapping only; not used for edit application).
- Invalid-edit rejection is all-or-nothing per `didChange` notification; partial application of a multi-change batch is not attempted (consistent with rejecting the whole batch on first failure).
