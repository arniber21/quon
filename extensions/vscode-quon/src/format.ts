import { spawn } from "child_process";
import * as vscode from "vscode";
import { BUILD_HINT, resolveFmtPath } from "./paths";

const COMMENT_WARNING_KEY = "quon.fmt.commentWarningShown";

async function maybeWarnAboutComments(context: vscode.ExtensionContext): Promise<void> {
  if (context.globalState.get(COMMENT_WARNING_KEY)) {
    return;
  }
  await context.globalState.update(COMMENT_WARNING_KEY, true);
  void vscode.window.showInformationMessage(
    "Quon: quonfmt v1 strips line and block comments. Format-on-save is off by default for this reason.",
  );
}

function runQuonfmt(bin: string, input: string): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(bin, [], { stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk: string) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk: string) => {
      stderr += chunk;
    });
    child.on("error", reject);
    child.on("close", (code) => {
      resolve({ code: code ?? 1, stdout, stderr });
    });
    child.stdin.write(input, "utf8");
    child.stdin.end();
  });
}

export function registerFormatter(context: vscode.ExtensionContext): vscode.Disposable {
  return vscode.languages.registerDocumentFormattingEditProvider("quon", {
    async provideDocumentFormattingEdits(
      document: vscode.TextDocument,
    ): Promise<vscode.TextEdit[]> {
      const bin = resolveFmtPath();
      if (!bin) {
        void vscode.window.showErrorMessage(
          `Quon: could not find quonfmt. Build with:\n${BUILD_HINT}\nOr set quon.fmt.path / QUON_FMT_PATH.`,
        );
        return [];
      }

      await maybeWarnAboutComments(context);

      try {
        const result = await runQuonfmt(bin, document.getText());
        if (result.code !== 0) {
          // quonfmt may exit 2 (plan) or 1 (anyhow-wrapped CLI); treat any non-zero
          // with stderr as a parse/format failure rather than only special-casing 2.
          const detail = result.stderr.trim();
          const isParseLike = result.code === 2 || detail.length > 0;
          void vscode.window.showErrorMessage(
            isParseLike
              ? `Quon: quonfmt parse error${detail ? `: ${detail}` : ""}`
              : `Quon: quonfmt failed (exit ${result.code})`,
          );
          return [];
        }
        const fullRange = new vscode.Range(
          document.positionAt(0),
          document.positionAt(document.getText().length),
        );
        return [vscode.TextEdit.replace(fullRange, result.stdout)];
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        void vscode.window.showErrorMessage(`Quon: failed to run quonfmt: ${msg}`);
        return [];
      }
    },
  });
}
