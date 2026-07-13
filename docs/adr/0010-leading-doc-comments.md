# Leading `--` / `{- -}` comments are symbol documentation

Documentation shown on hover and completion comes from ordinary leading comments
immediately above a top-level `fn` or `type` declaration — not from a dedicated
doc-comment syntax.

## Considered Options

**1. Leading ordinary comments (chosen).** Any `--` line comments or `{- -}` block
comments that appear immediately above a declaration (only whitespace between the
comment run and the `fn`/`type` keyword) become that symbol's docs. Recovered from
source text at symbol-index build time because the lexer skips comments and they
never enter the AST. Matches what users already write; no new syntax for v1.

**2. Dedicated doc syntax** (e.g. `---` lines or `{-| … -}`). Keeps ordinary
comments out of hover, but invents surface syntax and requires lexer/parser changes
before any tooling can use docs. Deferred unless leading comments prove too noisy.

## Consequences

- `Symbol.docs` is filled for top-level functions and type aliases; hover
  (`format_hover`) and completion (`documentation`) read the same field.
- **`quonfmt` v1 strips comments.** Formatting a file removes the source of LSP
  documentation. Documented in `docs/quonfmt-style.md` and `quonfmt/README.md`.
  Preserving comments in the formatter is a follow-up; until then, treat leading
  docs as editor-buffer metadata that does not survive a format pass.
- Inline / trailing comments and comments separated from a decl by other code do
  not attach. Blank lines between a leading comment run and the declaration still
  allow attachment.
