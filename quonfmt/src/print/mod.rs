pub mod decl;
pub mod expr;
pub mod nat;
pub mod pat;
pub mod stmt;
pub mod ty;

use crate::config::StyleConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    None,
    Circuit,
    Run,
    Borrow,
}

pub struct Context<'a> {
    pub config: &'a StyleConfig,
    pub indent_level: usize,
    pub block_kind: BlockKind,
}

impl<'a> Context<'a> {
    pub fn new(config: &'a StyleConfig) -> Self {
        Self {
            config,
            indent_level: 0,
            block_kind: BlockKind::None,
        }
    }

    pub fn nested(&self) -> Context<'a> {
        Context {
            config: self.config,
            indent_level: self.indent_level + 1,
            block_kind: self.block_kind,
        }
    }

    pub fn with_block(&self, kind: BlockKind) -> Context<'a> {
        Context {
            config: self.config,
            indent_level: self.indent_level,
            block_kind: kind,
        }
    }

    pub fn current_indent(&self) -> String {
        // `nested()` is always called before entering a block (CircuitBlock/
        // RunBlock/Borrow), so `indent_level` already reflects the block depth;
        // a top-level block body lives at level 1 → one indent (4 spaces),
        // not two. The prior `indent_level + 1` double-counted and emitted
        // 8-space bodies (contradicting quonfmt-style.md's "4 spaces").
        self.config.indent.repeat(self.indent_level)
    }
}

pub fn binop_str(op: frontend::ast::BinOp) -> &'static str {
    match op {
        frontend::ast::BinOp::Add => "+",
        frontend::ast::BinOp::Sub => "-",
        frontend::ast::BinOp::Mul => "*",
        frontend::ast::BinOp::Div => "/",
        frontend::ast::BinOp::Pow => "^",
    }
}

pub fn class_str(c: &frontend::ast::CliffordClass) -> &'static str {
    match c {
        frontend::ast::CliffordClass::Clifford => "Clifford",
        frontend::ast::CliffordClass::Universal => "Universal",
        frontend::ast::CliffordClass::Infer => "<unresolved-class>",
    }
}

pub fn render_float(f: f64) -> String {
    let s = format!("{f:?}");
    if s.contains('.')
        || s.contains('e')
        || s.contains('E')
        || s.contains("inf")
        || s.contains("NaN")
    {
        s
    } else {
        format!("{s}.0")
    }
}
