use crate::lexer::SimpleSpan;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScopeId(pub u32);

#[derive(Debug, Clone)]
pub struct Scope {
    pub id: ScopeId,
    pub parent: Option<ScopeId>,
    pub span: SimpleSpan,
    pub symbols: Vec<crate::analysis::symbols::SymbolId>,
}

#[derive(Debug)]
pub struct ScopeStack {
    scopes: Vec<Scope>,
    current: ScopeId,
}

impl ScopeStack {
    pub fn new(root_span: SimpleSpan) -> Self {
        let root = Scope {
            id: ScopeId(0),
            parent: None,
            span: root_span,
            symbols: Vec::new(),
        };
        Self {
            scopes: vec![root],
            current: ScopeId(0),
        }
    }

    pub fn current(&self) -> ScopeId {
        self.current
    }

    pub fn push(&mut self, span: SimpleSpan) -> ScopeId {
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Scope {
            id,
            parent: Some(self.current),
            span,
            symbols: Vec::new(),
        });
        self.current = id;
        id
    }

    pub fn pop(&mut self) {
        if let Some(scope) = self.scopes.get(self.current.0 as usize) {
            self.current = scope.parent.unwrap_or(ScopeId(0));
        }
    }

    pub fn add_symbol(&mut self, id: crate::analysis::symbols::SymbolId) {
        if let Some(scope) = self.scopes.get_mut(self.current.0 as usize) {
            scope.symbols.push(id);
        }
    }

    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }
}
