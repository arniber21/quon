;; synced from tree-sitter-quon/queries/brackets.scm — do not edit; run scripts/bump-zed-grammar-rev.sh
; Bracket pairs for Zed / editors that load brackets.scm.
; Requires anonymous delimiter tokens in grammar.js (not a lumped `delimiter` node).

("{" @open "}" @close)
("[" @open "]" @close)
("(" @open ")" @close)
("<" @open ">" @close)
