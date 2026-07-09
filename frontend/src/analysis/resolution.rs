use std::collections::HashMap;

use crate::analysis::symbols::SymbolId;
use crate::lexer::SimpleSpan;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTarget {
    Symbol(SymbolId),
    Builtin(String),
    Gate(String),
    QuantumBuiltin(String),
    TypeAlias(SymbolId),
}

/// Maps identifier use-site spans to their resolved definition target.
#[derive(Debug, Clone, Default)]
pub struct ResolutionMap {
    by_use_span: HashMap<(usize, usize), ResolvedTarget>,
}

impl ResolutionMap {
    pub fn record(&mut self, span: SimpleSpan, target: ResolvedTarget) {
        self.by_use_span.insert((span.start, span.end), target);
    }

    pub fn get(&self, span: SimpleSpan) -> Option<&ResolvedTarget> {
        self.by_use_span.get(&(span.start, span.end))
    }

    pub fn entries(&self) -> impl Iterator<Item = (SimpleSpan, &ResolvedTarget)> + '_ {
        self.by_use_span
            .iter()
            .map(|(&(start, end), target)| ((start..end).into(), target))
    }
}
