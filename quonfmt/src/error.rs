use thiserror::Error;

#[derive(Debug, Error)]
pub enum FormatError {
    #[error("parse failed")]
    Parse {
        diagnostics: Vec<frontend::diagnostics::Diagnostic>,
    },
    #[error("source is not formatted")]
    NotFormatted { expected: String },
}

impl FormatError {
    pub fn parse(diagnostics: Vec<frontend::diagnostics::Diagnostic>) -> Self {
        Self::Parse { diagnostics }
    }
}
