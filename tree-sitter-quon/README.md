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

## Lexical surface (keep TextMate in sync)

**Keywords:** `fn`, `type`, `let`, `in`, `return`, `match`, `circuit`, `run`, `borrow`, `for`, `if`, `then`, `else`, `true`, `false`, `adjoint`, `controlled`, `par`

**Comments:** line `-- …`, nested block `{- … -}`

**Operators:** `|>`, `<-`, `@`, `->`, `-o`, `=>`, and arithmetic / delimiters

Source of truth for the language lexer: `frontend/src/lexer.rs`.

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
| `queries/highlights.scm` | Zed, Neovim |
| `queries/brackets.scm` | Zed (pair matching). Requires anonymous `"{"` / `"}"` tokens in `grammar.js` — do **not** collapse delimiters into a named `delimiter` node |
| `queries/indents.scm` | Neovim / Zed (minimal stub) |
| `queries/locals.scm` | Optional / minimal |
