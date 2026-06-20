// Tokenizer — see issue #5, SPEC.md §2.
// A chumsky 0.13 lexer generator: produces Vec<Sp<Token>> from a UTF-8 source string.
// The `Token` enum is only the output type — all recognition and longest-match
// disambiguation is expressed with chumsky combinators below.
//
// Sp<T> = (T, SimpleSpan) is defined here and used throughout the frontend.
// chumsky 0.13 provides the `SimpleSpan` span type natively.

use chumsky::prelude::*;

pub type SimpleSpan = chumsky::span::SimpleSpan;
pub type Sp<T> = (T, SimpleSpan);

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Fn,
    Type,
    Let,
    In,
    Return,
    Match,
    Circuit,
    Run,
    Borrow,
    For,
    If,
    Then,
    Else,
    True,
    False,
    Adjoint,
    Controlled,
    Par,

    // Operators
    Pipe,        // |>
    Bind,        // <-
    At,          // @
    Arrow,       // ->
    LinearArrow, // -o
    Plus,        // +
    Minus,       // -
    Star,        // *
    Slash,       // /
    Caret,       // ^
    Eq,          // =
    FatArrow,    // =>
    Colon,       // :
    Comma,       // ,
    Dot,         // .
    Underscore,  // _
    Backtick,    // `
    Bar,         // | (match-arm alternation)

    // Delimiters
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    LAngle,
    RAngle,

    // Literals
    Int(i64),
    Float(f64),
    Bool(bool),

    // Identifiers
    Ident(String),

    // Significant newline (statement separator inside run/circuit/borrow blocks)
    Newline,

    // End of input
    Eof,
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Token::*;
        let s = match self {
            Fn => "fn",
            Type => "type",
            Let => "let",
            In => "in",
            Return => "return",
            Match => "match",
            Circuit => "circuit",
            Run => "run",
            Borrow => "borrow",
            For => "for",
            If => "if",
            Then => "then",
            Else => "else",
            True => "true",
            False => "false",
            Adjoint => "adjoint",
            Controlled => "controlled",
            Par => "par",
            Pipe => "|>",
            Bind => "<-",
            At => "@",
            Arrow => "->",
            LinearArrow => "-o",
            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            Caret => "^",
            Eq => "=",
            FatArrow => "=>",
            Colon => ":",
            Comma => ",",
            Dot => ".",
            Underscore => "_",
            Backtick => "`",
            Bar => "|",
            LBrace => "{",
            RBrace => "}",
            LParen => "(",
            RParen => ")",
            LBracket => "[",
            RBracket => "]",
            LAngle => "<",
            RAngle => ">",
            Int(n) => return write!(f, "{n}"),
            Float(x) => return write!(f, "{x}"),
            Bool(b) => return write!(f, "{b}"),
            Ident(name) => return write!(f, "{name}"),
            Newline => "newline",
            Eof => "end of input",
        };
        f.write_str(s)
    }
}

/// Tokenize `src` into a spanned token stream.
///
/// On success returns the tokens (no `Eof` is appended; the parser uses `end()`).
/// On failure returns one `(message, span)` per lexical error — never panics.
pub fn lex(src: &str) -> Result<Vec<Sp<Token>>, Vec<Sp<String>>> {
    lexer().parse(src).into_result().map_err(|errs| {
        errs.into_iter()
            .map(|e| (e.to_string(), *e.span()))
            .collect()
    })
}

type LexErr<'src> = extra::Err<Rich<'src, char>>;

fn lexer<'src>() -> impl Parser<'src, &'src str, Vec<Sp<Token>>, LexErr<'src>> {
    // ── Numeric literals ──────────────────────────────────────────────────────
    // Floats must be tried before ints so `1.5` is not lexed as `1 . 5`.
    // `text::digits(10)` is `[0-9]+` (leading zeros allowed, matching the spec).
    let exponent = one_of("eE")
        .then(one_of("+-").or_not())
        .then(text::digits(10))
        .to_slice();

    let float = text::digits(10)
        .then(just('.'))
        .then(text::digits(10))
        .then(exponent.or_not())
        .to_slice()
        .try_map(|s: &str, span| {
            s.parse::<f64>()
                .map(Token::Float)
                .map_err(|e| Rich::custom(span, format!("invalid float literal: {e}")))
        });

    let int = text::digits(10).to_slice().try_map(|s: &str, span| {
        s.parse::<i64>()
            .map(Token::Int)
            .map_err(|e| Rich::custom(span, format!("invalid integer literal: {e}")))
    });

    // ── Identifiers and keywords ──────────────────────────────────────────────
    // `_` alone is the wildcard token; `_foo` is an identifier.
    let ident = text::ascii::ident().map(|s: &str| match s {
        "fn" => Token::Fn,
        "type" => Token::Type,
        "let" => Token::Let,
        "in" => Token::In,
        "return" => Token::Return,
        "match" => Token::Match,
        "circuit" => Token::Circuit,
        "run" => Token::Run,
        "borrow" => Token::Borrow,
        "for" => Token::For,
        "if" => Token::If,
        "then" => Token::Then,
        "else" => Token::Else,
        "true" => Token::True,
        "false" => Token::False,
        "adjoint" => Token::Adjoint,
        "controlled" => Token::Controlled,
        "par" => Token::Par,
        "_" => Token::Underscore,
        other => Token::Ident(other.to_string()),
    });

    // ── Operators and punctuation ─────────────────────────────────────────────
    // Multi-character tokens come first so a prefix never wins the longest match.
    let op = choice((
        just("|>").to(Token::Pipe),
        just("<-").to(Token::Bind),
        just("->").to(Token::Arrow),
        just("-o").to(Token::LinearArrow),
        just("=>").to(Token::FatArrow),
        just('+').to(Token::Plus),
        just('-').to(Token::Minus),
        just('*').to(Token::Star),
        just('/').to(Token::Slash),
        just('^').to(Token::Caret),
        just('@').to(Token::At),
        just('=').to(Token::Eq),
        just(':').to(Token::Colon),
        just(',').to(Token::Comma),
        just('.').to(Token::Dot),
        just('`').to(Token::Backtick),
        just('|').to(Token::Bar),
        just('{').to(Token::LBrace),
        just('}').to(Token::RBrace),
        just('(').to(Token::LParen),
        just(')').to(Token::RParen),
        just('[').to(Token::LBracket),
        just(']').to(Token::RBracket),
        just('<').to(Token::LAngle),
        just('>').to(Token::RAngle),
    ));

    let token = choice((float, int, ident, op));

    // A run of newlines collapses to a single significant `Newline` token. A line
    // comment's terminating newline still counts.
    let newline = choice((just('\n'), just('\r')))
        .repeated()
        .at_least(1)
        .to(Token::Newline);

    let spanned = choice((newline, token)).map_with(|t, e| (t, e.span()));

    // ── Skipped whitespace and comments (never newlines) ──────────────────────
    let inline_ws = one_of(" \t").repeated().at_least(1).ignored();

    let line_comment = just("--")
        .then(any().and_is(just('\n').not()).repeated())
        .ignored();

    // Nested block comments: `{- ... {- ... -} ... -}`.
    let block_comment = recursive(|block| {
        let inner = choice((
            block,
            any()
                .and_is(just("-}").not())
                .and_is(just("{-").not())
                .ignored(),
        ));
        just("{-").then(inner.repeated()).then(just("-}")).ignored()
    });

    let skip = choice((inline_ws, line_comment, block_comment))
        .repeated()
        .ignored();

    // Leading skip handles whitespace/comments before the first token (and the
    // all-whitespace/empty file case); each token then consumes trailing skip.
    skip.clone()
        .ignore_then(
            spanned
                .then_ignore(skip.clone())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then_ignore(end())
}
