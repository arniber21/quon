import * as vscode from "vscode";
import { registerFormatter } from "./format";
import {
  getClient,
  getResolvedServerPath,
  restartLanguageClient,
  startLanguageClient,
  stopLanguageClient,
} from "./lsp";
import { BUILD_HINT, resolveFmtPath, resolveLspPath } from "./paths";

/** Public test/host API (returned from `activate`, available as `extension.exports`). */
export type QuonExtensionApi = {
  getClient: typeof getClient;
};

function activationErrorMessage(err: unknown): string {
  if (err instanceof Error) {
    return err.message;
  }
  return String(err);
}

export async function activate(context: vscode.ExtensionContext): Promise<QuonExtensionApi> {
  // Register commands and the formatter before starting the LSP so a missing or
  // crashing quon_lsp still leaves "Show Server Status" / Restart usable.
  context.subscriptions.push(registerFormatter(context));

  context.subscriptions.push(
    vscode.commands.registerCommand("quon.restartServer", async () => {
      try {
        await restartLanguageClient(context);
        void vscode.window.showInformationMessage("Quon: language server restarted.");
      } catch (err) {
        void vscode.window.showErrorMessage(
          `Quon: failed to restart language server: ${activationErrorMessage(err)}`,
        );
      }
    }),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("quon.showServerStatus", () => {
      const client = getClient();
      const running = client?.isRunning() ? "running" : "not running";
      const lsp = getResolvedServerPath() ?? resolveLspPath() ?? "(not found)";
      const fmt = resolveFmtPath() ?? "(not found)";
      void vscode.window.showInformationMessage(
        `Quon LSP (${running}): ${lsp}\nquonfmt: ${fmt}\nBuild hint: ${BUILD_HINT}`,
      );
    }),
  );

  try {
    await startLanguageClient(context);
  } catch (err) {
    // Keep the extension active (commands + formatter) even when the server fails.
    void vscode.window.showErrorMessage(
      `Quon: language server failed to start: ${activationErrorMessage(err)}\n` +
        `Build with: ${BUILD_HINT}\nOr set quon.lsp.path / QUON_LSP_PATH. ` +
        `See Output → "Quon Language Server" and "Extension Host".`,
    );
  }

  return { getClient };
}

export async function deactivate(): Promise<void> {
  await stopLanguageClient();
}
