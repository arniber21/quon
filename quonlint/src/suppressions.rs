use std::collections::{HashMap, HashSet};

/// Inline suppression directives parsed from `# quonlint-disable` comments.
#[derive(Debug, Clone, Default)]
pub struct SuppressionState {
    file_disabled: HashSet<String>,
    next_line: HashMap<usize, HashSet<String>>,
}

impl SuppressionState {
    pub fn parse(source: &str) -> Self {
        let mut state = Self::default();
        for (line_idx, line) in source.lines().enumerate() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("# quonlint-disable ") {
                let rule = rest.split_whitespace().next().unwrap_or("").to_string();
                if !rule.is_empty() {
                    state.file_disabled.insert(rule);
                }
            } else if let Some(rest) = trimmed.strip_prefix("# quonlint-disable-next-line ") {
                let rule = rest.split_whitespace().next().unwrap_or("").to_string();
                if !rule.is_empty() {
                    state
                        .next_line
                        .entry(line_idx + 1)
                        .or_default()
                        .insert(rule);
                }
            }
        }
        state
    }

    pub fn is_suppressed_at_line(&self, rule: &str, line: usize) -> bool {
        if self.file_disabled.contains(rule) {
            return true;
        }
        self.next_line
            .get(&line)
            .is_some_and(|rules| rules.contains(rule))
    }

    pub fn is_suppressed_with_source(&self, rule: &str, byte_offset: usize, source: &str) -> bool {
        if self.file_disabled.contains(rule) {
            return true;
        }
        let line = source[..byte_offset.min(source.len())]
            .bytes()
            .filter(|&b| b == b'\n')
            .count();
        self.is_suppressed_at_line(rule, line)
    }
}
