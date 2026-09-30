#!/usr/bin/env python3
"""Fail when editor highlight artifacts drift from tree-sitter-quon (issue #205).

Canonical editor sources:
  tree-sitter-quon/highlight-lexicon.json
      keyword, operator, punctuation, and delimiter vocabulary
  tree-sitter-quon/queries/{highlights,brackets,indents}.scm
      query text copied into the Zed extension (one sync header line prepended)

Not copies:
  tree-sitter-quon/queries/locals.scm     Neovim / tree-sitter locals only
  extensions/zed-quon/languages/quon/outline.scm
                                          Zed outline only

The compiler lexer is not generated. This script only checks that
frontend keyword and operator spellings still match the lexicon.
"""

from __future__ import annotations

import json
import re
import sys
import unittest
from pathlib import Path

LEXICON_REL = "tree-sitter-quon/highlight-lexicon.json"
GRAMMAR_REL = "tree-sitter-quon/grammar.js"
QUERIES_REL = "tree-sitter-quon/queries"
ZED_QUERIES_REL = "extensions/zed-quon/languages/quon"
ZED_TOML_REL = "extensions/zed-quon/extension.toml"
ZED_GITIGNORE_REL = "extensions/zed-quon/.gitignore"
TEXTMATE_REL = "extensions/vscode-quon/syntaxes/quon.tmLanguage.json"
PRELUDE_REL = "frontend/src/analysis/prelude_names.rs"
LEXER_REL = "frontend/src/lexer.rs"

COPY_QUERIES = ("highlights.scm", "brackets.scm", "indents.scm")
TREE_SITTER_ONLY_QUERIES = ("locals.scm",)
ZED_ONLY_QUERIES = ("outline.scm",)
ZED_OUTLINE_HEADER = (
    ";; Zed-only outline query. Not a copy of tree-sitter-quon/queries."
)

LEXICON_LIST_KEYS = (
    "surface_keywords",
    "declaration_keywords",
    "booleans",
    "rule_keywords",
    "operators",
    "punctuation",
    "delimiters",
)

SYNC_HEADER = (
    ";; synced from tree-sitter-quon/queries/{name} — do not edit; "
    "run scripts/bump-zed-grammar-rev.sh"
)

KEYWORD_ARM = re.compile(
    r'"([A-Za-z_][A-Za-z0-9_]*)"\s*=>\s*Token::([A-Za-z0-9_]+)'
)
DISPLAY_ARM = re.compile(
    r"\b([A-Za-z][A-Za-z0-9_]*)\s*=>\s*\"((?:\\.|[^\"\\])*)\""
)
PRELUDE_FN = re.compile(
    r"pub fn keywords\(\) -> &'static \[&'static str\] \{\s*&\[(.*?)\]\s*\}",
    re.DOTALL,
)
QUOTED = re.compile(r'"([^"]+)"')
REV_LINE = re.compile(r'(?m)^rev = "([0-9a-fA-F]{40})"$')


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def sync_header(name: str) -> str:
    return SYNC_HEADER.format(name=name)


def escape_atom(atom: str) -> str:
    """Escape one TextMate alternation atom. Hyphen is escaped only when lone."""
    if atom == "-":
        return r"\-"
    meta = set(r"\|.+?()[]{}^$*")
    return "".join("\\" + ch if ch in meta else ch for ch in atom)


def textmate_keyword_pattern(words: list[str]) -> str:
    return r"\b(" + "|".join(words) + r")\b"


def textmate_operator_pattern(operators: list[str], punctuation: list[str]) -> str:
    atoms = [escape_atom(atom) for atom in [*operators, *punctuation]]
    return "(" + "|".join(atoms) + ")"


def zed_query_text(name: str, canonical: str) -> str:
    if not canonical.endswith("\n"):
        canonical += "\n"
    return sync_header(name) + "\n" + canonical


def load_json(path: Path):
    with path.open(encoding="utf-8") as fh:
        return json.load(fh)


def lexicon_errors(lexicon: object) -> list[str]:
    errors: list[str] = []
    if not isinstance(lexicon, dict):
        return ["highlight-lexicon.json must be an object"]
    unknown = set(lexicon) - set(LEXICON_LIST_KEYS) - {"description"}
    if unknown:
        errors.append(
            "highlight-lexicon.json has unknown keys: " + ", ".join(sorted(unknown))
        )
    lists: dict[str, list[str]] = {}
    for key in LEXICON_LIST_KEYS:
        value = lexicon.get(key)
        if not isinstance(value, list) or not value or not all(
            isinstance(item, str) and item for item in value
        ):
            errors.append(f"{key} must be a non-empty list of strings")
            continue
        if len(value) != len(set(value)):
            errors.append(f"{key} contains duplicates")
        lists[key] = value
    if len(lists) != len(LEXICON_LIST_KEYS):
        return errors

    surface = lists["surface_keywords"]
    parts = (
        lists["declaration_keywords"]
        + lists["booleans"]
        + lists["rule_keywords"]
    )
    if set(parts) != set(surface):
        errors.append(
            "surface_keywords must equal declaration_keywords ∪ booleans ∪ rule_keywords"
        )
    if len(parts) != len(set(parts)):
        errors.append("declaration_keywords, booleans, and rule_keywords overlap")
    symbol_groups = ("operators", "punctuation", "delimiters")
    symbols: list[str] = []
    for key in symbol_groups:
        symbols.extend(lists[key])
    if len(symbols) != len(set(symbols)):
        errors.append("operators, punctuation, and delimiters overlap")
    if set(symbols) & set(surface):
        errors.append("a keyword is also listed as an operator, punctuation, or delimiter")
    return errors


def grammar_errors(grammar: str, lexicon: dict) -> list[str]:
    errors: list[str] = []
    if 'require("./highlight-lexicon.json")' not in grammar:
        errors.append('grammar.js must require("./highlight-lexicon.json")')
    for field in ("rule_keywords", "booleans", "operators", "punctuation"):
        needle = f"lexicon.{field}"
        if needle not in grammar:
            errors.append(f"grammar.js must spread {needle} into a token rule")
    for word in lexicon["declaration_keywords"]:
        if f'"{word}"' not in grammar:
            errors.append(
                f'grammar.js must mention declaration keyword "{word}" '
                "(fn/type stay literal seq tokens, not the keyword rule)"
            )
    for delim in lexicon["delimiters"]:
        if f'"{delim}"' not in grammar:
            errors.append(f'grammar.js must keep anonymous delimiter "{delim}"')
    return errors


def textmate_errors(textmate: dict, lexicon: dict) -> list[str]:
    errors: list[str] = []
    try:
        keyword = textmate["repository"]["keywords"]["patterns"][0]["match"]
        boolean = textmate["repository"]["booleans"]["patterns"][0]["match"]
        operator = textmate["repository"]["operators"]["patterns"][0]["match"]
    except (KeyError, IndexError, TypeError) as exc:
        return [f"TextMate grammar is missing keyword/boolean/operator patterns ({exc})"]

    keyword_words = [
        word
        for word in lexicon["surface_keywords"]
        if word not in lexicon["booleans"]
    ]
    expected_keyword = textmate_keyword_pattern(keyword_words)
    expected_boolean = textmate_keyword_pattern(lexicon["booleans"])
    expected_operator = textmate_operator_pattern(
        lexicon["operators"], lexicon["punctuation"]
    )
    if keyword != expected_keyword:
        errors.append(
            "TextMate keyword match drifted.\n"
            f"  expected: {expected_keyword}\n"
            f"  actual:   {keyword}"
        )
    if boolean != expected_boolean:
        errors.append(
            "TextMate boolean match drifted.\n"
            f"  expected: {expected_boolean}\n"
            f"  actual:   {boolean}"
        )
    if operator != expected_operator:
        errors.append(
            "TextMate operator match drifted.\n"
            f"  expected: {expected_operator}\n"
            f"  actual:   {operator}"
        )
    return errors


def prelude_keywords(text: str) -> list[str] | None:
    match = PRELUDE_FN.search(text)
    if not match:
        return None
    return QUOTED.findall(match.group(1))


def prelude_errors(text: str, lexicon: dict) -> list[str]:
    found = prelude_keywords(text)
    if found is None:
        return ["could not find prelude_names::keywords()"]
    expected = lexicon["surface_keywords"]
    if found != expected:
        return [
            "prelude_names::keywords() drifted from highlight-lexicon.json "
            f"surface_keywords.\n  expected: {expected}\n  actual:   {found}"
        ]
    return []


def lexer_errors(text: str, lexicon: dict) -> list[str]:
    errors: list[str] = []
    arms = {word: variant for word, variant in KEYWORD_ARM.findall(text)}
    arms.pop("_", None)
    expected = set(lexicon["surface_keywords"])
    if set(arms) != expected:
        missing = sorted(expected - set(arms))
        extra = sorted(set(arms) - expected)
        errors.append(
            "lexer keyword match drifted from surface_keywords "
            f"(missing {missing}, extra {extra})"
        )
    display = {literal: variant for variant, literal in DISPLAY_ARM.findall(text)}
    for word in lexicon["surface_keywords"]:
        if word not in display:
            errors.append(f'lexer Display is missing keyword "{word}"')
    surface = set(lexicon["surface_keywords"])
    symbol_expected = set(
        lexicon["operators"] + lexicon["punctuation"] + lexicon["delimiters"]
    )
    ignored_display = {"newline", "end of input"}
    symbol_actual = {
        literal
        for literal in display
        if literal not in surface and literal not in ignored_display
    }
    if symbol_actual != symbol_expected:
        missing = sorted(symbol_expected - symbol_actual)
        extra = sorted(symbol_actual - symbol_expected)
        errors.append(
            "lexer Display operators/punctuation/delimiters drifted "
            f"(missing {missing}, extra {extra})"
        )
    return errors


def query_errors(root: Path) -> list[str]:
    errors: list[str] = []
    query_dir = root / QUERIES_REL
    zed_dir = root / ZED_QUERIES_REL
    canonical_names = {path.name for path in query_dir.glob("*.scm")}
    expected_canonical = set(COPY_QUERIES) | set(TREE_SITTER_ONLY_QUERIES)
    if canonical_names != expected_canonical:
        errors.append(
            "tree-sitter-quon/queries/*.scm changed membership "
            f"(expected {sorted(expected_canonical)}, found {sorted(canonical_names)}). "
            "Classify the new query as a Zed copy or tree-sitter-only in "
            "scripts/check_editor_grammar_sync.py."
        )
    zed_names = {path.name for path in zed_dir.glob("*.scm")}
    expected_zed = set(COPY_QUERIES) | set(ZED_ONLY_QUERIES)
    if zed_names != expected_zed:
        errors.append(
            "extensions/zed-quon/languages/quon/*.scm changed membership "
            f"(expected {sorted(expected_zed)}, found {sorted(zed_names)})"
        )
    for name in COPY_QUERIES:
        canonical_path = query_dir / name
        zed_path = zed_dir / name
        if not canonical_path.is_file() or not zed_path.is_file():
            continue
        expected = zed_query_text(name, canonical_path.read_text(encoding="utf-8"))
        actual = zed_path.read_text(encoding="utf-8")
        if actual != expected:
            errors.append(
                f"{ZED_QUERIES_REL}/{name} drifted from {QUERIES_REL}/{name}. "
                "Run scripts/bump-zed-grammar-rev.sh (it rewrites the Zed copies)."
            )
    outline = zed_dir / "outline.scm"
    if outline.is_file():
        first = outline.read_text(encoding="utf-8").splitlines()[:1]
        if first != [ZED_OUTLINE_HEADER]:
            errors.append(
                "outline.scm must start with the Zed-only header "
                f"(got {first!r})"
            )
    return errors


def sync_zed_queries(root: Path) -> None:
    query_dir = root / QUERIES_REL
    zed_dir = root / ZED_QUERIES_REL
    for name in COPY_QUERIES:
        canonical = (query_dir / name).read_text(encoding="utf-8")
        (zed_dir / name).write_text(zed_query_text(name, canonical), encoding="utf-8")


def rev_errors(text: str) -> list[str]:
    if not REV_LINE.search(text):
        return [
            'extensions/zed-quon/extension.toml must contain rev = "<40 hex chars>"'
        ]
    return []


def gitignore_errors(text: str) -> list[str]:
    errors: list[str] = []
    for needle in ("/grammars/", "*.wasm"):
        if needle not in text:
            errors.append(
                f"extensions/zed-quon/.gitignore must ignore Zed build product {needle}"
            )
    return errors


def lockfile_errors(root: Path) -> list[str]:
    errors: list[str] = []
    required = (
        "tree-sitter-quon/package-lock.json",
        "extensions/vscode-quon/package-lock.json",
        "website/pnpm-lock.yaml",
    )
    forbidden = (
        "tree-sitter-quon/pnpm-lock.yaml",
        "tree-sitter-quon/yarn.lock",
        "extensions/vscode-quon/pnpm-lock.yaml",
        "extensions/vscode-quon/yarn.lock",
    )
    for rel in required:
        if not (root / rel).is_file():
            errors.append(f"missing lockfile required by the editor package policy: {rel}")
    for rel in forbidden:
        if (root / rel).is_file():
            errors.append(
                f"{rel} violates the package-manager policy "
                "(tree-sitter-quon and extensions/vscode-quon use npm; website uses pnpm)"
            )
    return errors


def collect_errors(root: Path) -> list[str]:
    errors: list[str] = []
    lexicon_path = root / LEXICON_REL
    try:
        lexicon = load_json(lexicon_path)
    except (OSError, json.JSONDecodeError) as exc:
        return [f"cannot read {LEXICON_REL}: {exc}"]
    errors.extend(lexicon_errors(lexicon))
    if any(key not in lexicon for key in LEXICON_LIST_KEYS):
        return errors

    def read(rel: str) -> str | None:
        path = root / rel
        if not path.is_file():
            errors.append(f"missing {rel}")
            return None
        return path.read_text(encoding="utf-8")

    grammar = read(GRAMMAR_REL)
    if grammar is not None:
        errors.extend(grammar_errors(grammar, lexicon))
    textmate_text = read(TEXTMATE_REL)
    if textmate_text is not None:
        try:
            textmate = json.loads(textmate_text)
        except json.JSONDecodeError as exc:
            errors.append(f"cannot parse {TEXTMATE_REL}: {exc}")
        else:
            errors.extend(textmate_errors(textmate, lexicon))
    prelude = read(PRELUDE_REL)
    if prelude is not None:
        errors.extend(prelude_errors(prelude, lexicon))
    lexer = read(LEXER_REL)
    if lexer is not None:
        errors.extend(lexer_errors(lexer, lexicon))
    errors.extend(query_errors(root))
    toml = read(ZED_TOML_REL)
    if toml is not None:
        errors.extend(rev_errors(toml))
    gitignore = read(ZED_GITIGNORE_REL)
    if gitignore is not None:
        errors.extend(gitignore_errors(gitignore))
    errors.extend(lockfile_errors(root))
    return errors


class SyncUnitTests(unittest.TestCase):
    def test_escape_atoms(self) -> None:
        self.assertEqual(escape_atom("|>"), r"\|>")
        self.assertEqual(escape_atom("-"), r"\-")
        self.assertEqual(escape_atom("-o"), "-o")
        self.assertEqual(escape_atom("`"), "`")
        self.assertEqual(escape_atom("|"), r"\|")
        self.assertEqual(escape_atom("."), r"\.")

    def test_textmate_patterns(self) -> None:
        self.assertEqual(
            textmate_keyword_pattern(["fn", "let"]),
            r"\b(fn|let)\b",
        )
        self.assertEqual(
            textmate_operator_pattern(["|>", "-"], ["."]),
            r"(\|>|\-|\.)",
        )

    def test_zed_query_text_prepends_header(self) -> None:
        text = zed_query_text("highlights.scm", "; body\n")
        self.assertTrue(text.startswith(sync_header("highlights.scm") + "\n"))
        self.assertTrue(text.endswith("; body\n"))

    def test_lexicon_overlap_is_reported(self) -> None:
        lexicon = {
            "description": "x",
            "surface_keywords": ["fn", "let"],
            "declaration_keywords": ["fn"],
            "booleans": ["fn"],
            "rule_keywords": ["let"],
            "operators": ["+"],
            "punctuation": ["+"],
            "delimiters": ["{"],
        }
        messages = " ".join(lexicon_errors(lexicon))
        self.assertIn("overlap", messages)


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(SyncUnitTests)
        result = unittest.TextTestRunner(verbosity=1).run(suite)
        return 0 if result.wasSuccessful() else 1
    root = repo_root()
    if "--sync-zed" in argv:
        sync_zed_queries(root)
        print("synced Zed queries from tree-sitter-quon/queries/")
        return 0
    errors = collect_errors(root)
    if errors:
        print("editor grammar sync: FAIL", file=sys.stderr)
        for message in errors:
            print(f"- {message}", file=sys.stderr)
        return 1
    print("editor grammar sync: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
