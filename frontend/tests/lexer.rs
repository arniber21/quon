// Lexer unit tests — issue #5 acceptance criteria.

use frontend::lexer::{Token, lex};

/// Lex `src` and return just the token kinds (spans stripped), panicking on lex error.
fn toks(src: &str) -> Vec<Token> {
    lex(src)
        .unwrap_or_else(|e| panic!("lex error on {src:?}: {e:?}"))
        .into_iter()
        .map(|(t, _)| t)
        .collect()
}

#[test]
fn all_keywords() {
    use Token::*;
    let src = "fn type let in return match circuit run borrow for if then else true false adjoint controlled par";
    assert_eq!(
        toks(src),
        vec![
            Fn, Type, Let, In, Return, Match, Circuit, Run, Borrow, For, If, Then, Else, True,
            False, Adjoint, Controlled, Par,
        ]
    );
}

#[test]
fn identifiers_are_not_keywords() {
    use Token::*;
    // Stdlib/gate names are plain identifiers, including keyword-prefixed ones.
    assert_eq!(
        toks("H CNOT Rz measure qreg fold PI function letx"),
        vec![
            Ident("H".into()),
            Ident("CNOT".into()),
            Ident("Rz".into()),
            Ident("measure".into()),
            Ident("qreg".into()),
            Ident("fold".into()),
            Ident("PI".into()),
            Ident("function".into()),
            Ident("letx".into()),
        ]
    );
}

#[test]
fn multichar_operator_disambiguation() {
    use Token::*;
    assert_eq!(toks("|>"), vec![Pipe]);
    assert_eq!(toks("|"), vec![Bar]);
    assert_eq!(toks("<-"), vec![Bind]);
    assert_eq!(toks("<"), vec![LAngle]);
    assert_eq!(toks("->"), vec![Arrow]);
    assert_eq!(toks("-o"), vec![LinearArrow]);
    assert_eq!(toks("-"), vec![Minus]);
    assert_eq!(toks("=>"), vec![FatArrow]);
    assert_eq!(toks("="), vec![Eq]);
    // No conflict between a multi-char token and its single-char prefix.
    assert_eq!(
        toks("- > - o"),
        vec![Minus, RAngle, Minus, Ident("o".into())]
    );
}

#[test]
fn all_punctuation() {
    use Token::*;
    assert_eq!(
        toks("@ + * / ^ : , . ` _ { } ( ) [ ] < >"),
        vec![
            At, Plus, Star, Slash, Caret, Colon, Comma, Dot, Backtick, Underscore, LBrace, RBrace,
            LParen, RParen, LBracket, RBracket, LAngle, RAngle,
        ]
    );
}

#[test]
fn underscore_vs_underscore_ident() {
    use Token::*;
    assert_eq!(toks("_"), vec![Underscore]);
    assert_eq!(toks("_rest"), vec![Ident("_rest".into())]);
    assert_eq!(toks("_ _rest"), vec![Underscore, Ident("_rest".into())]);
}

#[test]
fn integer_literals() {
    use Token::*;
    assert_eq!(toks("0 42 007"), vec![Int(0), Int(42), Int(7)]);
}

#[test]
fn float_literals() {
    use Token::*;
    assert_eq!(toks("1.5"), vec![Float(1.5)]);
    assert_eq!(toks("2.0"), vec![Float(2.0)]);
    assert_eq!(toks("3.141592653589793"), vec![Float(std::f64::consts::PI)]);
    assert_eq!(toks("1.0e10"), vec![Float(1.0e10)]);
    assert_eq!(toks("6.02e+23"), vec![Float(6.02e23)]);
    assert_eq!(toks("1.5e-3"), vec![Float(1.5e-3)]);
}

#[test]
fn float_before_int_no_dot_split() {
    use Token::*;
    // `1.5` is one float, not `1 . 5`; `q.f` (no digits after dot) is `q . f`.
    assert_eq!(toks("1.5"), vec![Float(1.5)]);
    assert_eq!(toks("q.f"), vec![Ident("q".into()), Dot, Ident("f".into())]);
    // `5.` with no fractional digits is an int then a dot.
    assert_eq!(toks("5 . 0"), vec![Int(5), Dot, Int(0)]);
}

#[test]
fn line_comments_skipped() {
    use Token::*;
    assert_eq!(
        toks("a -- this is ignored\nb"),
        vec![Ident("a".into()), Newline, Ident("b".into())]
    );
    // A line comment with no trailing newline at EOF.
    assert_eq!(toks("x -- trailing"), vec![Ident("x".into())]);
}

#[test]
fn block_comments_skipped_and_nested() {
    use Token::*;
    assert_eq!(
        toks("a {- comment -} b"),
        vec![Ident("a".into()), Ident("b".into())]
    );
    assert_eq!(
        toks("a {- outer {- inner -} still -} b"),
        vec![Ident("a".into()), Ident("b".into())]
    );
}

#[test]
fn significant_newlines_collapse() {
    use Token::*;
    // Runs of newlines collapse to a single Newline; leading/trailing newlines kept.
    assert_eq!(
        toks("a\n\n\nb"),
        vec![Ident("a".into()), Newline, Ident("b".into())]
    );
    assert_eq!(toks("\n  a  \n"), vec![Newline, Ident("a".into()), Newline]);
}

#[test]
fn inline_whitespace_skipped() {
    use Token::*;
    assert_eq!(
        toks("  a\t\tb  "),
        vec![Ident("a".into()), Ident("b".into())]
    );
}

#[test]
fn empty_and_whitespace_only() {
    assert_eq!(toks(""), vec![]);
    assert_eq!(toks("   \t  "), vec![]);
}

#[test]
fn bell_state_circuit_line() {
    use Token::*;
    assert_eq!(
        toks("H @0 |> CNOT @(0, 1)"),
        vec![
            Ident("H".into()),
            At,
            Int(0),
            Pipe,
            Ident("CNOT".into()),
            At,
            LParen,
            Int(0),
            Comma,
            Int(1),
            RParen,
        ]
    );
}

#[test]
fn unknown_char_is_span_accurate_error_not_panic() {
    // `#` is not a valid token; lexing must return Err with a span, never panic.
    let err = lex("a # b").expect_err("expected lex error");
    assert!(!err.is_empty());
    let (_, span) = &err[0];
    // The offending `#` is at byte offset 2.
    assert_eq!(span.start, 2);
}
