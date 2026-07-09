import * as vscode from "vscode";
import { registerFormatter } from "./format";
import {
  getResolvedServerPath,
  restartLanguageClient,
  startLanguageClient,
  stopLanguageClient,
} from "./lsp";
import { BUILD_HINT, resolveFmtPath, resolveLspPath } from "./paths";

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  context.subscriptions.push(registerFormatter(context));

  context.subscriptions.push(
    vscode.commands.registerCommand("quon.restartServer", async () => {
      await restartLanguageClient(context);
      void vscode.window.showInformationMessage("Quon: language server restarted.");
    }),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("quon.showServerStatus", () => {
      const lsp = getResolvedServerPath() ?? resolveLspPath() ?? "(not found)";
      const fmt = resolveFmtPath() ?? "(not found)";
      void vscode.window.showInformationMessage(
        `Quon LSP: ${lsp}\nquonfmt: ${fmt}\nBuild hint: ${BUILD_HINT}`,
      );
    }),
  );

  await startLanguageClient(context);
}

export async function deactivate(): Promise<void> {
  await stopLanguageClient();
}
