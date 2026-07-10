;; synced from tree-sitter-quon/queries/brackets.scm — update both
; Bracket pairs for Zed. Requires anonymous "{" / "}" tokens in grammar.js.

("{" @open "}" @close)
("[" @open "]" @close)
("(" @open ")" @close)
("<" @open ">" @close)
