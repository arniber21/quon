// Tokenizer — see issue #5, SPEC.md §2
// Produces Vec<Sp<Token>> from a UTF-8 source string.
// Sp<T> = (T, SimpleSpan) defined in this module.

use chumsky::span::SimpleSpan;

pub type Sp<T> = (T, SimpleSpan);

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Fn, Type, Let, In, Return, Match,
    Circuit, Run, Borrow, For, If, Then, Else,
    True, False, Adjoint, Controlled, Par,

    // Operators
    Pipe,        // |>
    Bind,        // <-
    At,          // @
    Arrow,       // ->
    LinearArrow, // -o
    Star,        // *
    Caret,       // ^
    Eq,          // =
    Colon,       // :
    Comma,       // ,
    Underscore,  // _

    // Delimiters
    LBrace, RBrace,
    LParen, RParen,
    LAngle, RAngle,

    // Literals
    Int(i64),
    Float(f64),
    Bool(bool),

    // Identifiers
    Ident(String),

    // End of input
    Eof,
}

pub fn lex(_src: &str) -> Result<Vec<Sp<Token>>, Vec<Sp<String>>> {
    todo!("lexer — see issue #5")
}
