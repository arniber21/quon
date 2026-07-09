import * as vscode from "vscode";

export type QuonLogLevel = "off" | "error" | "warn" | "info" | "debug" | "trace";
export type QuonTrace = "off" | "messages" | "verbose";

export interface QuonSettings {
  lspPath: string;
  debounceMs: number;
  logLevel: QuonLogLevel;
  trace: QuonTrace;
  fmtPath: string;
  formatOnSave: boolean;
  lintConfigPath: string;
}

export function readSettings(): QuonSettings {
  const c = vscode.workspace.getConfiguration("quon");
  return {
    lspPath: c.get<string>("lsp.path", ""),
    debounceMs: c.get<number>("lsp.debounceMs", 100),
    logLevel: c.get<QuonLogLevel>("lsp.logLevel", "info"),
    trace: c.get<QuonTrace>("lsp.trace", "off"),
    fmtPath: c.get<string>("fmt.path", ""),
    formatOnSave: c.get<boolean>("fmt.formatOnSave", false),
    lintConfigPath: c.get<string>("lint.configPath", ""),
  };
}

/** Env vars passed to the quon_lsp child process. */
export function serverEnv(settings: QuonSettings): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = { ...process.env };
  env.QUON_LSP_DEBOUNCE_MS = String(settings.debounceMs);
  if (settings.logLevel === "off") {
    env.QUON_LOG = "off";
    env.RUST_LOG = "off";
  } else {
    env.QUON_LOG = settings.logLevel;
    env.RUST_LOG = `quon_lsp=${settings.logLevel}`;
  }
  return env;
}
