;; synced from tree-sitter-quon/queries/highlights.scm — update both
; Highlight queries for Quon (Zed / Neovim).
; Keep in sync with the keyword / operator list in README.md and TextMate.

(line_comment) @comment
(block_comment) @comment

(fn_declaration
  "fn" @keyword
  name: (identifier) @function)

(type_declaration
  "type" @keyword
  name: (identifier) @type)

(keyword) @keyword
(boolean) @boolean
(number) @number
(identifier) @variable
(operator) @operator

"{" @punctuation.bracket
"}" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
"(" @punctuation.bracket
")" @punctuation.bracket
"<" @punctuation.bracket
">" @punctuation.bracket

(punctuation) @punctuation.delimiter
