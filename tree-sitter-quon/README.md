# tree-sitter-quon

Canonical [Tree-sitter](https://tree-sitter.github.io/tree-sitter/) grammar for the Quon language (`.qn`).

Owned initially by [#131](https://github.com/arniber21/quon/issues/131) (VS Code). Consumed by [#132](https://github.com/arniber21/quon/issues/132) (Zed) and [#133](https://github.com/arniber21/quon/issues/133) (Neovim).

## Consumption contract

```text
Canonical grammar: /tree-sitter-quon
Corpus (tree-sitter test): /tree-sitter-quon/test/corpus/   # CLI standard — do not use /corpus at package root
Zed (#132): point grammar path at ../../tree-sitter-quon (or copy queries into extension as build step)
Neovim (#133): nvim-treesitter local parser or :TSInstall from path
Do not fork grammar.js
Do not relocate or duplicate the corpus directory
```

This package is a **grammar source** (`grammar.js`, committed `src/parser.c`, `queries/`). `package.json` `"main"` is `index.js` (path metadata only — **not** a native `bindings/node` addon). Zed/Neovim should consume this directory by path for the Tree-sitter CLI / parser C sources.

**Hard rules for #132 / #133:**

- Do **not** invent a second `grammar.js` or fork `src/parser.c`.
- Corpus lives at **`tree-sitter-quon/test/corpus/`** only (tree-sitter CLI default).
- Prefer relative path into this package from editor extensions in the monorepo.

## VS Code note

The VS Code extension (`extensions/vscode-quon/`) uses **TextMate** for lexical highlighting plus LSP semantic tokens. Tree-sitter is the canonical grammar for Zed/Neovim; embedding Tree-sitter WASM in VS Code is optional and not required for #131.

## Lexical surface

Editor highlight vocabulary lives in [`highlight-lexicon.json`](highlight-lexicon.json). `grammar.js` token rules read that file. Do not hand-copy keyword lists into TextMate, Zed queries, or docs — `scripts/check_editor_grammar_sync.py` (also `just ci-editor-grammar`) fails if those copies drift, including `frontend` `keywords()` and the lexer display strings. The compiler is not generated from the lexicon.

**Comments:** line `-- …`, nested block `{- … -}`

## Package manager

This package uses npm (`package-lock.json`, `npm ci`), same as `extensions/vscode-quon/`. The website stays on pnpm. Do not commit `pnpm-lock.yaml` or `yarn.lock` here.

## Build / test

```sh
cd tree-sitter-quon
npm ci
npx tree-sitter generate
npx tree-sitter test
```

Generated `src/parser.c` is committed so consumers do not need the CLI at runtime.

## Queries

| Query | Consumers |
| ----- | --------- |
| `queries/highlights.scm` | Canonical. Zed copy: `extensions/zed-quon/languages/quon/highlights.scm` (rewritten by `scripts/bump-zed-grammar-rev.sh`) |
| `queries/brackets.scm` | Canonical. Zed copy, same command. Requires anonymous `"{"` / `"}"` tokens in `grammar.js` — do **not** collapse delimiters into a named `delimiter` node |
| `queries/indents.scm` | Canonical. Zed copy, same command |
| `queries/locals.scm` | Tree-sitter / Neovim only. Not copied into Zed |
