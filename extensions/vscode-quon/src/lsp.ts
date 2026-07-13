import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  Trace,
} from "vscode-languageclient/node";
import { readSettings, serverEnv, QuonTrace } from "./config";
import { BUILD_HINT, resolveLspPath } from "./paths";

let client: LanguageClient | undefined;
let lastResolvedPath: string | undefined;

function mapTrace(trace: QuonTrace): Trace {
  switch (trace) {
    case "messages":
      return Trace.Messages;
    case "verbose":
      return Trace.Verbose;
    default:
      return Trace.Off;
  }
}

export function getClient(): LanguageClient | undefined {
  return client;
}

export function getResolvedServerPath(): string | undefined {
  return lastResolvedPath;
}

export async function startLanguageClient(context: vscode.ExtensionContext): Promise<LanguageClient | undefined> {
  const settings = readSettings();
  const command = resolveLspPath();
  if (!command) {
    void vscode.window.showErrorMessage(
      `Quon: could not find quon_lsp. Build with:\n${BUILD_HINT}\nOr set quon.lsp.path / QUON_LSP_PATH.`,
    );
    return undefined;
  }
  lastResolvedPath = command;

  const serverOptions: ServerOptions = {
    command,
    args: [],
    options: {
      env: serverEnv(settings),
    },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "quon" },
      { scheme: "untitled", language: "quon" },
    ],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.qn"),
    },
    // Formatting: keep the extension's DocumentFormattingEditProvider (format.ts).
    // quon_lsp also advertises textDocument/formatting (#178). Do **not** enable
    // both for the same buffer — double-format hazard. Default formatter stays
    // `quon.quon-vscode` (see package.json configurationDefaults).
    outputChannelName: "Quon Language Server",
  };

  client = new LanguageClient("quon", "Quon Language Server", serverOptions, clientOptions);
  client.setTrace(mapTrace(settings.trace));
  context.subscriptions.push(client);
  try {
    await client.start();
  } catch (err) {
    // Drop the failed client so status/restart reflect reality and a later
    // restart can construct a fresh LanguageClient.
    client = undefined;
    lastResolvedPath = command;
    throw err;
  }
  return client;
}

export async function stopLanguageClient(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}

export async function restartLanguageClient(context: vscode.ExtensionContext): Promise<void> {
  await stopLanguageClient();
  await startLanguageClient(context);
}
