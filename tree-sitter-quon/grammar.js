/**
 * Highlighting-grade Tree-sitter grammar for Quon.
 * Lexical surface mirrors frontend/src/lexer.rs — not a second frontend parser.
 *
 * Structure is intentionally loose (error-tolerant) so highlights.scm can paint
 * keywords, comments, numbers, and operators without full type fidelity.
 *
 * Delimiters are anonymous string tokens (`"{"`, `"}"`, …) so Zed/Neovim
 * brackets.scm can match them. Do not collapse them into a single `delimiter`
 * named token — that makes `("{" @open "}" @close)` fail with
 * "Invalid node type `{`".
 */
module.exports = grammar({
  name: "quon",

  extras: ($) => [/\s+/],

  word: ($) => $.identifier,

  rules: {
    source_file: ($) => repeat($._node),

    _node: ($) =>
      choice(
        $.line_comment,
        $.block_comment,
        $.fn_declaration,
        $.type_declaration,
        $.keyword,
        $.boolean,
        $.number,
        $.identifier,
        $.operator,
        "{",
        "}",
        "[",
        "]",
        "(",
        ")",
        "<",
        ">",
        $.punctuation,
      ),

    fn_declaration: ($) =>
      seq("fn", field("name", $.identifier)),

    type_declaration: ($) =>
      seq("type", field("name", $.identifier)),

    keyword: (_) =>
      token(
        choice(
          "circuit",
          "run",
          "borrow",
          "par",
          "match",
          "let",
          "in",
          "return",
          "for",
          "if",
          "then",
          "else",
          "adjoint",
          "controlled",
        ),
      ),

    boolean: (_) => token(choice("true", "false")),

    number: (_) =>
      token(choice(/-?\d+\.\d+([eE][+-]?\d+)?/, /-?\d+([eE][+-]?\d+)?/)),

    identifier: (_) => /[A-Za-z_][A-Za-z0-9_]*/,

    operator: (_) =>
      token(choice("|>", "<-", "->", "-o", "=>", "@", "=", "+", "-", "*", "/", "^", "|")),

    punctuation: (_) => token(choice(":", ",", ".", "_", "`")),

    line_comment: (_) => token(seq("--", /[^\n]*/)),

    // Nested block comments: {- outer {- inner -} still -}
    block_comment: (_) =>
      token(
        seq(
          "{-",
          repeat(
            choice(
              /[^{-]/,
              /\{[^-]/,
              /-[^}]/,
              seq("{-", repeat(choice(/[^{-]/, /\{[^-]/, /-[^}]/)), "-}"),
            ),
          ),
          "-}",
        ),
      ),
  },
});
