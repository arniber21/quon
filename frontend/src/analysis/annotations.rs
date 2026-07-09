use std::collections::HashMap;

use crate::lexer::SimpleSpan;
use crate::types::Ty;

/// Inferred types keyed by the span of an expression or binding site.
#[derive(Debug, Clone, Default)]
pub struct TypeAnnotations {
    by_span: HashMap<(usize, usize), Ty>,
}

impl TypeAnnotations {
    pub fn record(&mut self, span: SimpleSpan, ty: Ty) {
        self.by_span.insert((span.start, span.end), ty);
    }

    pub fn get(&self, span: SimpleSpan) -> Option<&Ty> {
        self.by_span.get(&(span.start, span.end))
    }

    /// Iterate all recorded expression types (for lint export).
    pub fn iter(&self) -> impl Iterator<Item = ((usize, usize), &Ty)> {
        self.by_span.iter().map(|(k, v)| (*k, v))
    }
}
