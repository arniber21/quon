use std::fmt;
use std::str::FromStr;

use frontend::lexer::SimpleSpan;
use serde::{Deserialize, Serialize};

pub type RuleId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Allow,
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn is_emitted(self, min: Severity) -> bool {
        self != Severity::Allow && self >= min
    }

    pub fn fails_build(self, threshold: Severity) -> bool {
        self != Severity::Allow && self >= threshold
    }
}

impl FromStr for Severity {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "allow" | "off" => Ok(Self::Allow),
            "info" | "note" => Ok(Self::Info),
            "warn" | "warning" => Ok(Self::Warning),
            "error" => Ok(Self::Error),
            _ => Err(()),
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Allow => f.write_str("allow"),
            Self::Info => f.write_str("info"),
            Self::Warning => f.write_str("warning"),
            Self::Error => f.write_str("error"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanJson {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintDiagnostic {
    pub rule: RuleId,
    pub severity: Severity,
    pub message: String,
    pub span: SpanJson,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
}

impl LintDiagnostic {
    pub fn new(
        rule: impl Into<RuleId>,
        severity: Severity,
        message: impl Into<String>,
        span: SimpleSpan,
    ) -> Self {
        Self {
            rule: rule.into(),
            severity,
            message: message.into(),
            span: SpanJson {
                start: span.start,
                end: span.end,
            },
            help: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn simple_span(&self) -> SimpleSpan {
        (self.span.start..self.span.end).into()
    }
}
