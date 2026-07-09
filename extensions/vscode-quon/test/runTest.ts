import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { runTests } from "@vscode/test-electron";

/** Absolutize env paths so the Extension Host (different cwd) can exec them. */
function absolutizeEnvPath(value: string | undefined): string | undefined {
  if (!value || value.length === 0) {
    return value;
  }
  if (path.isAbsolute(value)) {
    return value;
  }
  return path.resolve(process.cwd(), value);
}

async function main() {
  // extensions/vscode-quon/ → repo root (../..)
  const extensionDevelopmentPath = path.resolve(__dirname, "../../");
  const extensionTestsPath = path.resolve(__dirname, "./suite/index");
  const repoRoot = path.resolve(extensionDevelopmentPath, "../..");

  // Keep user-data under /tmp so the IPC socket path stays under the OS limit
  // (deep worktree paths otherwise break VS Code on macOS/Linux).
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "quon-vscode-ud-"));

  const lspPath = absolutizeEnvPath(process.env.QUON_LSP_PATH);
  const fmtPath = absolutizeEnvPath(process.env.QUON_FMT_PATH);

  await runTests({
    extensionDevelopmentPath,
    extensionTestsPath,
    // Propagate binary discovery env into the Extension Host.
    extensionTestsEnv: {
      QUON_LSP_PATH: lspPath,
      QUON_FMT_PATH: fmtPath,
      QUON_LSP_DEBOUNCE_MS: process.env.QUON_LSP_DEBOUNCE_MS ?? "0",
    },
    // Open the monorepo root so relative fixture paths resolve
    launchArgs: [repoRoot, "--disable-extensions", `--user-data-dir=${userDataDir}`],
  });
}

main().catch((err) => {
  console.error("Failed to run extension tests");
  console.error(err);
  process.exit(1);
});
