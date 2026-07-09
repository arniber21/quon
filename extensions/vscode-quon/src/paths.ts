import * as fs from "fs";
import * as path from "path";
import { execFileSync } from "child_process";
import * as vscode from "vscode";

export const BUILD_HINT =
  "cargo build --release -p quon_lsp -p quonfmt -p quonlint-cli";

function isExecutable(filePath: string): boolean {
  try {
    fs.accessSync(filePath, fs.constants.X_OK);
    return fs.statSync(filePath).isFile();
  } catch {
    return false;
  }
}

function resolveMaybeRelative(candidate: string, workspaceFolders: readonly vscode.WorkspaceFolder[]): string | undefined {
  if (!candidate) {
    return undefined;
  }
  if (path.isAbsolute(candidate) && isExecutable(candidate)) {
    return candidate;
  }
  for (const folder of workspaceFolders) {
    const joined = path.resolve(folder.uri.fsPath, candidate);
    if (isExecutable(joined)) {
      return joined;
    }
  }
  if (isExecutable(candidate)) {
    return candidate;
  }
  return undefined;
}

function which(binary: string): string | undefined {
  try {
    const cmd = process.platform === "win32" ? "where" : "which";
    const out = execFileSync(cmd, [binary], { encoding: "utf8" }).trim();
    const first = out.split(/\r?\n/)[0]?.trim();
    if (first && isExecutable(first)) {
      return first;
    }
  } catch {
    // not on PATH
  }
  return undefined;
}

function workspaceTarget(binary: string, workspaceFolders: readonly vscode.WorkspaceFolder[]): string | undefined {
  for (const folder of workspaceFolders) {
    for (const profile of ["release", "debug"] as const) {
      const candidate = path.join(folder.uri.fsPath, "target", profile, binary);
      if (isExecutable(candidate)) {
        return candidate;
      }
      if (process.platform === "win32") {
        const win = `${candidate}.exe`;
        if (isExecutable(win)) {
          return win;
        }
      }
    }
  }
  return undefined;
}

export type ToolKind = "lsp" | "fmt";

/**
 * Symmetric discovery for quon_lsp / quonfmt:
 * setting → env (QUON_LSP_PATH / QUON_FMT_PATH) → PATH → target/{release,debug}
 */
export function resolveTool(kind: ToolKind): string | undefined {
  const config = vscode.workspace.getConfiguration("quon");
  const folders = vscode.workspace.workspaceFolders ?? [];
  const binary = kind === "lsp" ? "quon_lsp" : "quonfmt";
  const settingKey = kind === "lsp" ? "lsp.path" : "fmt.path";
  const envKey = kind === "lsp" ? "QUON_LSP_PATH" : "QUON_FMT_PATH";

  const fromSetting = resolveMaybeRelative(config.get<string>(settingKey, ""), folders);
  if (fromSetting) {
    return fromSetting;
  }

  const envVal = process.env[envKey];
  if (envVal && envVal.length > 0) {
    const fromEnv = resolveMaybeRelative(envVal, folders);
    if (fromEnv) {
      return fromEnv;
    }
    if (isExecutable(envVal)) {
      return envVal;
    }
  }

  const fromPath = which(binary);
  if (fromPath) {
    return fromPath;
  }

  return workspaceTarget(binary, folders);
}

export function resolveLspPath(): string | undefined {
  return resolveTool("lsp");
}

export function resolveFmtPath(): string | undefined {
  return resolveTool("fmt");
}
