; Bracket pairs for Zed / editors that load brackets.scm.
; Requires anonymous delimiter tokens in grammar.js (not a lumped `delimiter` node).

("{" @open "}" @close)
("[" @open "]" @close)
("(" @open ")" @close)
("<" @open ">" @close)
