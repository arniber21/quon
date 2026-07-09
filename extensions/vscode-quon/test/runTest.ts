import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { runTests } from "@vscode/test-electron";

async function main() {
  // extensions/vscode-quon/ → repo root (../..)
  const extensionDevelopmentPath = path.resolve(__dirname, "../../");
  const extensionTestsPath = path.resolve(__dirname, "./suite/index");
  const repoRoot = path.resolve(extensionDevelopmentPath, "../..");

  // Keep user-data under /tmp so the IPC socket path stays under the OS limit
  // (deep worktree paths otherwise break VS Code on macOS/Linux).
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "quon-vscode-ud-"));

  await runTests({
    extensionDevelopmentPath,
    extensionTestsPath,
    // Propagate binary discovery env into the Extension Host.
    extensionTestsEnv: {
      QUON_LSP_PATH: process.env.QUON_LSP_PATH,
      QUON_FMT_PATH: process.env.QUON_FMT_PATH,
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
