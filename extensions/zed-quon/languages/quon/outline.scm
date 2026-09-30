;; Zed-only outline query. Not a copy of tree-sitter-quon/queries.

(fn_declaration
  name: (identifier) @name) @item

(type_declaration
  name: (identifier) @name) @item
