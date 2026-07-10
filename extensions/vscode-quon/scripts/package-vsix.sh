#!/usr/bin/env bash
# Thin wrapper around vsce package.
set -euo pipefail
cd "$(dirname "$0")/.."
npm run compile
mkdir -p dist
npx vsce package --out dist/quon-vscode.vsix

# Guard against the syntax-only failure mode: TextMate works without activate(),
# but hover/diagnostics need vscode-languageclient inside the .vsix.
# Avoid `grep -q` under `pipefail` (SIGPIPE from early close can fail the pipeline).
listing="$(unzip -l dist/quon-vscode.vsix)"
if ! grep -F 'extension/node_modules/vscode-languageclient/' <<<"$listing" >/dev/null; then
  echo "error: packaged .vsix is missing vscode-languageclient (check .vscodeignore)" >&2
  exit 1
fi
echo "ok: vscode-languageclient present in dist/quon-vscode.vsix"
