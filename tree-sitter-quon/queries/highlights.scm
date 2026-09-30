; Highlight queries for Quon (Zed / Neovim).
; Keyword and operator vocabulary: highlight-lexicon.json (do not duplicate it here).

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
