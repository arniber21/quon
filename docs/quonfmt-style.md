# Quon canonical formatter style (quonfmt v1)

Normative layout rules for the `quonfmt` tool. Rust sources use `cargo fmt`; Quon
sources use `quonfmt`.

## Scope and principles

- UTF-8 source; LF line endings in output (CRLF normalized on read).
- Deterministic: fixed style constants in code (no user config file in v1).
- Correctness oracle: `parse(format(src))` equals `parse(src)` up to spans.
- Comments (`--` and `{- -}`) are stripped on format (not preserved in v1).

## Lexical formatting

| Topic | Rule |
| ----- | ---- |
| Indentation | 4 spaces; no tabs |
| Line endings | LF |
| Trailing whitespace | Stripped |
| Final newline | Required |
| Max line width | 100 columns (Unicode scalar counts) |
| Binary ops | One space: `+ - * / ^` |
| `@` | Space before `@`; tight target: `H @0`, `CNOT @(0, 1)` |
| `\|>` | One space each side; long chains break with leading `\|>` on continuations |
| `<-` | One space each side |
| `->` / `-o` | One space each side in types and lambdas |
| Unary `-` | Space after minus: `- x` |
| Commas | `,` + single space |
| Trailing comma | Omitted in single-line lists |

## Top-level declarations

- One blank line between declarations.
- Function: `fn name(params): Ret = body`; break after `=` when over width.
- Type alias: `type Name = …` or `type Name<n> = …`.

## Statement blocks

`circuit { }`, `run { }`, and `borrow … in { }` use one statement per line (+4 indent).

`run { }` bind alignment: when a block has 2+ `<-` binds, align `<-` columns.

## Expression blocks

`par { }` and `for … { }` contain a single expression body, not a statement list.

## Application (canonical re-surface)

| AST | Printed as |
| --- | ---------- |
| Juxtaposition `App(f, x)` | `f x` when both sides are atom-safe |
| Multi-arg call | `f(a, b, …)` |
| `App(f, Unit)` | `f()` |
| Index desugar | `index(q, i)` |
| Method/backtick desugar | `f(x, y)` |

## Configuration constants

```rust
INDENT = "    "
MAX_WIDTH = 100
DECL_SEP = "\n\n"
```
