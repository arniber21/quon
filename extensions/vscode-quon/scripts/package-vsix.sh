#!/usr/bin/env bash
# Thin wrapper around vsce package.
set -euo pipefail
cd "$(dirname "$0")/.."
npm run compile
mkdir -p dist
npx vsce package --out dist/quon-vscode.vsix
