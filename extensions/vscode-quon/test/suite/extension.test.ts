import * as assert from "assert";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

const BORROW_DISCARD_SRC =
  "fn f(): Q<Int> = run {\n  borrow a: Qubit in {\n    return 0\n  }\n}";

type QuonExtensionApi = {
  getClient: () => LanguageClient | undefined;
};

function requireWorkspaceRoot(): string {
  const folders = vscode.workspace.workspaceFolders;
  if (!folders || folders.length === 0) {
    throw new Error(
      "workspaceFolders is empty — runTest.ts must pass repoRoot in launchArgs",
    );
  }
  return folders[0].uri.fsPath;
}

function requireEnvBinary(name: "QUON_LSP_PATH" | "QUON_FMT_PATH"): string {
  const p = process.env[name];
  if (!p || !fs.existsSync(p)) {
    throw new Error(`${name} is missing or not executable: ${p ?? "(unset)"}`);
  }
  return p;
}

async function sleep(ms: number): Promise<void> {
  await new Promise((r) => setTimeout(r, ms));
}

async function waitFor(
  predicate: () => boolean | Promise<boolean>,
  label: string,
  timeoutMs = 20_000,
): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await predicate()) {
      return;
    }
    await sleep(200);
  }
  throw new Error(`timeout waiting for: ${label}`);
}

/** Resolve the activated extension API (LanguageClient handle). */
async function requireQuonApi(): Promise<QuonExtensionApi> {
  const ext = vscode.extensions.getExtension<QuonExtensionApi>("quon.quon-vscode");
  assert.ok(ext, "quon.quon-vscode extension not found");
  const api = ext.isActive ? ext.exports : await ext.activate();
  assert.ok(api && typeof api.getClient === "function", "extension exports missing getClient");
  return api;
}

/** Wait until the language client is running (proves LSP started — not vacuous []). */
async function waitForLspRunning(api: QuonExtensionApi): Promise<LanguageClient> {
  await waitFor(() => {
    const client = api.getClient();
    return !!client && client.isRunning();
  }, "quon_lsp LanguageClient running");
  const client = api.getClient();
  assert.ok(client && client.isRunning(), "LanguageClient not running after wait");
  return client;
}

/**
 * Wait until the server has responded for `uri`.
 *
 * `getDiagnostics(uri)` starts as `[]` before any publish, so `diags.every(...)`
 * is vacuous and must not be used alone. We subscribe *before* `open`, then accept:
 * - a diagnostics change event for this URI (including empty publish), or
 * - non-empty diagnostics (publish already applied), or
 * - a successful hover at `probePos` (proves analysis completed for clean files
 *   when an empty→empty republish might not fire a change event).
 */
async function waitUntilServerResponded(
  uri: vscode.Uri,
  open: () => Promise<vscode.Position | undefined | void>,
  timeoutMs = 20_000,
): Promise<readonly vscode.Diagnostic[]> {
  let published = false;
  let probePos: vscode.Position | undefined;
  const sub = vscode.languages.onDidChangeDiagnostics((e) => {
    if (e.uris.some((u) => u.toString() === uri.toString())) {
      published = true;
    }
  });
  try {
    const maybePos = await open();
    if (maybePos instanceof vscode.Position) {
      probePos = maybePos;
    }
    await waitFor(async () => {
      if (published) {
        return true;
      }
      if (vscode.languages.getDiagnostics(uri).length > 0) {
        return true;
      }
      if (probePos) {
        const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
          "vscode.executeHoverProvider",
          uri,
          probePos,
        );
        if (hovers && hovers.length > 0) {
          return true;
        }
      }
      return false;
    }, `LSP response for ${path.basename(uri.fsPath)}`, timeoutMs);
    return vscode.languages.getDiagnostics(uri);
  } finally {
    sub.dispose();
  }
}

async function openTempQuon(content: string): Promise<vscode.TextDocument> {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "quon-vscode-"));
  const file = path.join(dir, "tmp.qn");
  fs.writeFileSync(file, content, "utf8");
  const doc = await vscode.workspace.openTextDocument(vscode.Uri.file(file));
  await vscode.languages.setTextDocumentLanguage(doc, "quon");
  await vscode.window.showTextDocument(doc);
  return doc;
}

suite("Quon VS Code extension", () => {
  suiteSetup(() => {
    requireEnvBinary("QUON_LSP_PATH");
    requireEnvBinary("QUON_FMT_PATH");
    requireWorkspaceRoot();
  });

  test("bell_state.qn activates as quon with no error diagnostics", async () => {
    const api = await requireQuonApi();
    await waitForLspRunning(api);

    const root = requireWorkspaceRoot();
    const uri = vscode.Uri.file(path.join(root, "frontend/tests/fixtures/bell_state.qn"));

    const diags = await waitUntilServerResponded(uri, async () => {
      const doc = await vscode.workspace.openTextDocument(uri);
      await vscode.window.showTextDocument(doc);
      assert.strictEqual(doc.languageId, "quon");
      const declIdx = doc.lineAt(0).text.indexOf("bell_state");
      assert.ok(declIdx >= 0, "bell_state decl not found");
      return new vscode.Position(0, declIdx + 2);
    });

    const errors = diags.filter((d) => d.severity === vscode.DiagnosticSeverity.Error);
    assert.strictEqual(errors.length, 0, `unexpected errors: ${JSON.stringify(errors)}`);
  });

  test("type error produces diagnostics", async () => {
    const api = await requireQuonApi();
    await waitForLspRunning(api);

    const doc = await openTempQuon("fn bad(): Int = true\n");
    await waitFor(() => {
      return vscode.languages.getDiagnostics(doc.uri).length > 0;
    }, "type-error diagnostics");
    const diags = vscode.languages.getDiagnostics(doc.uri);
    assert.ok(diags.length > 0, "expected at least one diagnostic");
  });

  test("hover, definition, and completion", async () => {
    const api = await requireQuonApi();
    await waitForLspRunning(api);

    // Definition: local let-binding (stable LSP contract from quon_lsp intel tests).
    const defSrc = "fn f(): Int = let x = 1 in x\n";
    const defUri = vscode.Uri.file(
      path.join(fs.mkdtempSync(path.join(os.tmpdir(), "quon-vscode-")), "def.qn"),
    );
    fs.writeFileSync(defUri.fsPath, defSrc, "utf8");
    const useIdx = defSrc.lastIndexOf("x");
    const usePos = new vscode.Position(0, useIdx);

    const defDoc = await (async () => {
      let doc!: vscode.TextDocument;
      await waitUntilServerResponded(defUri, async () => {
        doc = await vscode.workspace.openTextDocument(defUri);
        await vscode.languages.setTextDocumentLanguage(doc, "quon");
        await vscode.window.showTextDocument(doc);
        return usePos;
      });
      return doc;
    })();
    const defDiags = vscode.languages.getDiagnostics(defDoc.uri);
    assert.ok(
      defDiags.every((x) => x.severity !== vscode.DiagnosticSeverity.Error),
      `unexpected errors before definition: ${JSON.stringify(defDiags)}`,
    );
    const defs = await vscode.commands.executeCommand<
      (vscode.Location | vscode.LocationLink)[]
    >("vscode.executeDefinitionProvider", defDoc.uri, usePos);
    assert.ok(defs && defs.length > 0, "expected definition results for local x");

    // Hover + completion on bell_state fixture.
    const root = requireWorkspaceRoot();
    const uri = vscode.Uri.file(path.join(root, "frontend/tests/fixtures/bell_state.qn"));
    let doc!: vscode.TextDocument;
    let namePos = new vscode.Position(0, 0);
    await waitUntilServerResponded(uri, async () => {
      doc = await vscode.workspace.openTextDocument(uri);
      await vscode.window.showTextDocument(doc);
      const declIdx = doc.lineAt(0).text.indexOf("bell_state");
      assert.ok(declIdx >= 0, "bell_state decl not found");
      namePos = new vscode.Position(0, declIdx + 2);
      return namePos;
    });

    const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
      "vscode.executeHoverProvider",
      uri,
      namePos,
    );
    assert.ok(hovers && hovers.length > 0, "expected hover results");

    let atPos: vscode.Position | undefined;
    for (let i = 0; i < doc.lineCount; i++) {
      const line = doc.lineAt(i).text;
      const idx = line.indexOf("@");
      if (idx >= 0) {
        atPos = new vscode.Position(i, idx + 1);
        break;
      }
    }
    assert.ok(atPos, "@ site not found");
    const completions = await vscode.commands.executeCommand<vscode.CompletionList>(
      "vscode.executeCompletionItemProvider",
      uri,
      atPos,
    );
    const items = completions?.items ?? [];
    assert.ok(items.length > 0, "expected completion items");
  });

  test("code action returns discard(a) quick-fix", async () => {
    const api = await requireQuonApi();
    await waitForLspRunning(api);

    const doc = await openTempQuon(BORROW_DISCARD_SRC);
    await waitFor(() => vscode.languages.getDiagnostics(doc.uri).length > 0, "borrow diagnostics");
    const diags = vscode.languages.getDiagnostics(doc.uri);
    assert.ok(diags.length > 0);
    const range = diags[0].range;

    const actions = await vscode.commands.executeCommand<(vscode.CodeAction | vscode.Command)[]>(
      "vscode.executeCodeActionProvider",
      doc.uri,
      range,
    );
    assert.ok(actions && actions.length > 0, "expected code actions");
    const titles = actions.map((a) => ("title" in a ? a.title : String(a)));
    assert.ok(
      titles.some((t) => t.includes("discard(a)")),
      `expected discard(a) in ${JSON.stringify(titles)}`,
    );
  });

  test("format document via quonfmt provider", async () => {
    const messy = "fn f():Int=1\n";
    const doc = await openTempQuon(messy);
    const edits = await vscode.commands.executeCommand<vscode.TextEdit[]>(
      "vscode.executeFormatDocumentProvider",
      doc.uri,
      { tabSize: 4, insertSpaces: true },
    );
    assert.ok(edits && edits.length > 0, "expected format edits");
    const formatted = applyEdits(doc.getText(), edits);
    // quonfmt style: spaces around : and =
    assert.ok(
      formatted.includes("fn f(): Int = 1") || formatted.trim() === "fn f(): Int = 1",
      `unexpected format result: ${JSON.stringify(formatted)}`,
    );
  });
});

function applyEdits(text: string, edits: vscode.TextEdit[]): string {
  // Single full-document replace is the common case for our provider.
  if (edits.length === 1) {
    return edits[0].newText;
  }
  const sorted = [...edits].sort(
    (a, b) => b.range.start.compareTo(a.range.start),
  );
  let result = text;
  for (const edit of sorted) {
    const start = offsetAt(result, edit.range.start);
    const end = offsetAt(result, edit.range.end);
    result = result.slice(0, start) + edit.newText + result.slice(end);
  }
  return result;
}

function offsetAt(text: string, pos: vscode.Position): number {
  const lines = text.split(/\n/);
  let offset = 0;
  for (let i = 0; i < pos.line; i++) {
    offset += lines[i].length + 1;
  }
  return offset + pos.character;
}
