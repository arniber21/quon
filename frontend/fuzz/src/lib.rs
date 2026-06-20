//! Shared code for the frontend fuzz targets: an `arbitrary`-driven AST generator that
//! produces syntactically valid (though not type-correct) Quon programs. Spans are all
//! `0..0`; identifiers and literals are constrained so the printed source re-lexes/parses.

pub mod gen;
