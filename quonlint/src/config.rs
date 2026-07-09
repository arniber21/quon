use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Deserialize;
use thiserror::Error;

use crate::diagnostic::{RuleId, Severity};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config {}: {0}", path.display())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("unknown severity `{0}`")]
    UnknownSeverity(String),
    #[error("unknown rule `{0}`")]
    UnknownRule(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    quonlint: QuonlintSection,
    #[serde(default)]
    rules: RulesSection,
    /// Root-level include globs (accepted for `quonlint.toml` convenience).
    #[serde(default)]
    include: Vec<String>,
    /// Root-level exclude globs (accepted for `quonlint.toml` convenience).
    #[serde(default)]
    exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuonlintSection {
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    fail_on: Option<String>,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default)]
    target: Option<PathBuf>,
    #[serde(default)]
    deep: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
// No deny_unknown_fields: serde rejects every key when the only field is
// `flatten` (all keys look "unknown"), which breaks `[rules]` tables.
struct RulesSection {
    #[serde(flatten)]
    entries: HashMap<String, toml::Value>,
}

/// Runtime lint configuration (defaults + file + CLI overrides).
#[derive(Debug, Clone)]
pub struct LintConfig {
    pub min_severity: Severity,
    pub fail_on: Severity,
    pub rule_severities: HashMap<RuleId, Severity>,
    pub disabled_rules: Vec<RuleId>,
    pub only_rules: Option<Vec<RuleId>>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub target_path: Option<PathBuf>,
    pub deep: bool,
    pub config_path: Option<PathBuf>,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            min_severity: Severity::Info,
            fail_on: Severity::Error,
            rule_severities: HashMap::new(),
            disabled_rules: Vec::new(),
            only_rules: None,
            include: vec!["**/*.qn".into()],
            exclude: vec!["**/target/**".into(), "website/**".into()],
            target_path: None,
            deep: false,
            config_path: None,
        }
    }
}

impl LintConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let file: FileConfig = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        let mut cfg = Self {
            config_path: Some(path.to_path_buf()),
            ..Self::default()
        };
        cfg.apply_file(&file)?;
        Ok(cfg)
    }

    pub fn discover_for_file(file: &Path) -> Self {
        if let Some(path) = find_config_upward(file) {
            Self::load(&path).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn discover_project(root: &Path) -> Self {
        for name in ["quonlint.toml", ".quonlintrc.toml"] {
            let path = root.join(name);
            if path.is_file() {
                return Self::load(&path).unwrap_or_default();
            }
        }
        Self::default()
    }

    pub fn with_fail_on(mut self, sev: Severity) -> Self {
        self.fail_on = sev;
        self
    }

    pub fn with_min_severity(mut self, sev: Severity) -> Self {
        self.min_severity = sev;
        self
    }

    pub fn with_only_rules(mut self, rules: Vec<RuleId>) -> Self {
        self.only_rules = Some(rules);
        self
    }

    pub fn with_except_rules(mut self, rules: Vec<RuleId>) -> Self {
        self.disabled_rules.extend(rules);
        self
    }

    pub fn with_deep(mut self, deep: bool) -> Self {
        self.deep = deep;
        self
    }

    pub fn effective_severity(&self, rule: &RuleId, default: Severity) -> Severity {
        if self.disabled_rules.contains(rule) {
            return Severity::Allow;
        }
        if let Some(sev) = self.rule_severities.get(rule) {
            *sev
        } else {
            default
        }
    }

    pub fn rule_enabled(&self, rule: &RuleId) -> bool {
        if self.disabled_rules.contains(rule) {
            return false;
        }
        if let Some(only) = &self.only_rules {
            return only.contains(rule);
        }
        true
    }

    fn apply_file(&mut self, file: &FileConfig) -> Result<(), ConfigError> {
        if let Some(level) = &file.quonlint.level {
            self.min_severity = parse_severity(level)?;
        }
        if let Some(fail_on) = &file.quonlint.fail_on {
            self.fail_on = parse_severity(fail_on)?;
        }
        if !file.quonlint.include.is_empty() {
            self.include = file.quonlint.include.clone();
        } else if !file.include.is_empty() {
            self.include = file.include.clone();
        }
        if !file.quonlint.exclude.is_empty() {
            self.exclude = file.quonlint.exclude.clone();
        } else if !file.exclude.is_empty() {
            self.exclude = file.exclude.clone();
        }
        self.target_path = file.quonlint.target.clone();
        self.deep = file.quonlint.deep;

        for (key, value) in &file.rules.entries {
            if value.is_table() {
                if let Some(sev) = value.get("severity").and_then(|v| v.as_str()) {
                    self.rule_severities
                        .insert(key.clone(), parse_severity(sev)?);
                }
            } else if let Some(sev) = value.as_str() {
                let severity = parse_severity(sev)?;
                if severity == Severity::Allow {
                    self.disabled_rules.push(key.clone());
                } else {
                    self.rule_severities.insert(key.clone(), severity);
                }
            }
        }
        Ok(())
    }
}

fn parse_severity(s: &str) -> Result<Severity, ConfigError> {
    Severity::from_str(s).map_err(|_| ConfigError::UnknownSeverity(s.to_string()))
}

fn find_config_upward(start: &Path) -> Option<PathBuf> {
    let mut dir = start.parent()?.to_path_buf();
    loop {
        for name in ["quonlint.toml", ".quonlintrc.toml"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        if !dir.pop() {
            break;
        }
    }
    None
}
