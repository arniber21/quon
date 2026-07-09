import * as assert from "assert";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as vscode from "vscode";

const BORROW_DISCARD_SRC =
  "fn f(): Q<Int> = run {\n  borrow a: Qubit in {\n    return 0\n  }\n}";

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
    const root = requireWorkspaceRoot();
    const uri = vscode.Uri.file(path.join(root, "frontend/tests/fixtures/bell_state.qn"));
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);
    assert.strictEqual(doc.languageId, "quon");

    await waitFor(() => {
      const diags = vscode.languages.getDiagnostics(uri);
      // Wait until LSP has published at least once (empty array is fine for clean file)
      // or until a short settle after open.
      return diags.every((d) => d.severity !== vscode.DiagnosticSeverity.Error);
    }, "no error diagnostics on bell_state");

    // Give the server a moment after open; then assert no errors.
    await sleep(1500);
    const diags = vscode.languages.getDiagnostics(uri);
    const errors = diags.filter((d) => d.severity === vscode.DiagnosticSeverity.Error);
    assert.strictEqual(errors.length, 0, `unexpected errors: ${JSON.stringify(errors)}`);
  });

  test("type error produces diagnostics", async () => {
    const doc = await openTempQuon("fn bad(): Int = true\n");
    await waitFor(() => {
      return vscode.languages.getDiagnostics(doc.uri).length > 0;
    }, "type-error diagnostics");
    const diags = vscode.languages.getDiagnostics(doc.uri);
    assert.ok(diags.length > 0, "expected at least one diagnostic");
  });

  test("hover, definition, and completion", async () => {
    // Definition: local let-binding (stable LSP contract from quon_lsp intel tests).
    const defDoc = await openTempQuon("fn f(): Int = let x = 1 in x\n");
    await waitFor(() => {
      // Wait until analysis has settled (no errors expected).
      const d = vscode.languages.getDiagnostics(defDoc.uri);
      return d.every((x) => x.severity !== vscode.DiagnosticSeverity.Error);
    }, "clean analysis for definition fixture");
    await sleep(500);
    const defText = defDoc.getText();
    const useIdx = defText.lastIndexOf("x");
    assert.ok(useIdx >= 0);
    const usePos = defDoc.positionAt(useIdx);
    const defs = await vscode.commands.executeCommand<
      (vscode.Location | vscode.LocationLink)[]
    >("vscode.executeDefinitionProvider", defDoc.uri, usePos);
    assert.ok(defs && defs.length > 0, "expected definition results for local x");

    // Hover + completion on bell_state fixture.
    const root = requireWorkspaceRoot();
    const uri = vscode.Uri.file(path.join(root, "frontend/tests/fixtures/bell_state.qn"));
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);
    await sleep(1000);

    const declLine = doc.lineAt(0).text;
    const declIdx = declLine.indexOf("bell_state");
    assert.ok(declIdx >= 0, "bell_state decl not found");
    const namePos = new vscode.Position(0, declIdx + 2);

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
