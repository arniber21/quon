//! Fixed style constants for the canonical formatter (v1 — no user config file).

/// Formatter configuration. Defaults match `docs/quonfmt-style.md`.
#[derive(Debug, Clone)]
pub struct StyleConfig {
    pub indent: &'static str,
    pub max_width: usize,
    pub decl_sep: &'static str,
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            indent: "    ",
            max_width: 100,
            decl_sep: "\n\n",
        }
    }
}

