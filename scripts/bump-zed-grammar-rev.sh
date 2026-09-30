#!/usr/bin/env bash
# One-command Zed grammar pin update (issue #205).
#
# 1. Rewrite extensions/zed-quon/languages/quon/{highlights,brackets,indents}.scm
#    from tree-sitter-quon/queries/ (sync header prepended).
# 2. Set [grammars.quon].rev in extensions/zed-quon/extension.toml.
#
# Usage:
#   scripts/bump-zed-grammar-rev.sh [REV]
#
# REV is a 40-character commit SHA that already contains the tree-sitter-quon/
# you want Zed to clone. It defaults to HEAD. Zed fetches that commit from
# GitHub, so bump only after the grammar commit exists on the remote (or pass
# the SHA of that commit). Reinstall the Zed Dev Extension after bumping.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
rev="${1:-$(git -C "$root" rev-parse HEAD)}"

if [[ ! "$rev" =~ ^[0-9a-fA-F]{40}$ ]]; then
  echo "error: rev must be a 40-character commit SHA (got: $rev)" >&2
  echo "usage: scripts/bump-zed-grammar-rev.sh [REV]" >&2
  exit 2
fi

python3 "$root/scripts/check_editor_grammar_sync.py" --sync-zed

toml="$root/extensions/zed-quon/extension.toml"
python3 - "$toml" "$rev" <<'PY'
import pathlib, re, sys
path, rev = sys.argv[1], sys.argv[2]
file = pathlib.Path(path)
text = file.read_text(encoding="utf-8")
new, count = re.subn(
    r'(?m)^rev = "[0-9a-fA-F]{40}"$',
    f'rev = "{rev}"',
    text,
    count=1,
)
if count != 1:
    sys.exit("could not find a 40-character rev = line in extension.toml")
file.write_text(new, encoding="utf-8")
print(f"set [grammars.quon].rev = {rev}")
PY

echo "Zed queries synced and grammar rev pinned."
echo "Reinstall the Zed Dev Extension so it reclones tree-sitter-quon at this rev."
