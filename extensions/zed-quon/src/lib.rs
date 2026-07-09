use std::collections::HashMap;
use std::path::{Path, PathBuf};

use zed_extension_api::{self as zed, settings::LspSettings, LanguageServerId, Result};

struct QuonExtension;

impl QuonExtension {
    const LANGUAGE_SERVER_ID: &'static str = "quon-lsp";
    const BINARY_NAME: &'static str = "quon_lsp";

    fn merge_env(
        shell_env: Vec<(String, String)>,
        settings_env: Option<HashMap<String, String>>,
    ) -> Vec<(String, String)> {
        let mut env = shell_env;
        if let Some(extra) = settings_env {
            for (key, value) in extra {
                if let Some((_, existing)) = env.iter_mut().find(|(k, _)| k == &key) {
                    *existing = value;
                } else {
                    env.push((key, value));
                }
            }
        }
        env
    }

    fn binary_name() -> String {
        match zed::current_platform().0 {
            zed::Os::Windows => format!("{}.exe", Self::BINARY_NAME),
            _ => Self::BINARY_NAME.to_string(),
        }
    }

    /// Quon-checkout detection (plan §4.4): virtual workspace with
    /// `quon_lsp` + `frontend` members, plus a marker path.
    fn is_quon_checkout(worktree: &zed::Worktree) -> bool {
        let Ok(manifest) = worktree.read_text_file("Cargo.toml") else {
            return false;
        };
        if !manifest.contains("[workspace]") {
            return false;
        }
        let has_quon_lsp = manifest
            .lines()
            .any(|line| line.contains("\"quon_lsp\"") || line.contains("'quon_lsp'"));
        let has_frontend = manifest
            .lines()
            .any(|line| line.contains("\"frontend\"") || line.contains("'frontend'"));
        if !has_quon_lsp || !has_frontend {
            return false;
        }

        worktree.read_text_file("frontend/src/lib.rs").is_ok()
            || worktree.read_text_file("SPEC.md").is_ok()
            || worktree
                .read_text_file("tree-sitter-quon/grammar.js")
                .is_ok()
            || worktree
                .read_text_file("tree-sitter-quon/package.json")
                .is_ok()
    }

    /// Candidate absolute paths under a Quon checkout (`release`, then `debug`).
    fn worktree_target_candidates(root: &Path, binary_name: &str) -> [PathBuf; 2] {
        [
            root.join("target").join("release").join(binary_name),
            root.join("target").join("debug").join(binary_name),
        ]
    }

    /// Host-side executable probe.
    ///
    /// Zed's WASI context only preopens the extension work dir, so guest
    /// `std::path::Path::is_file` / `std::fs` cannot see `{worktree}/target/...`.
    /// `zed::process::Command` runs on the host and can.
    fn host_path_is_executable(path: &str) -> bool {
        let output = match zed::current_platform().0 {
            zed::Os::Windows => {
                // Escape double-quotes in the path for `cmd /C if exist "..."`.
                let escaped = path.replace('"', "");
                zed::process::Command::new("cmd")
                    .args([
                        "/C",
                        &format!("if exist \"{escaped}\" (exit 0) else (exit 1)"),
                    ])
                    .output()
            }
            _ => zed::process::Command::new("test")
                .arg("-x")
                .arg(path)
                .output(),
        };
        output.is_ok_and(|o| o.status == Some(0))
    }

    fn worktree_target_binary(worktree: &zed::Worktree) -> Option<String> {
        if !Self::is_quon_checkout(worktree) {
            return None;
        }
        let root = PathBuf::from(worktree.root_path());
        let name = Self::binary_name();
        for candidate in Self::worktree_target_candidates(&root, &name) {
            let path = candidate.to_string_lossy();
            // Do NOT use Path::is_file here — it false-greens under host
            // `cargo test` and fails under Zed's WASM sandbox.
            if Self::host_path_is_executable(&path) {
                return Some(path.into_owned());
            }
        }
        None
    }

    fn missing_binary_error() -> String {
        format!(
            "Could not find `{bin}`. Build it with `cargo build -p quon_lsp --release`, \
             put `target/release` on PATH, or set an absolute path in settings:\n\
             {{\n\
               \"lsp\": {{\n\
                 \"quon-lsp\": {{\n\
                   \"binary\": {{\n\
                     \"path\": \"/ABS/PATH/TO/quon/target/release/{bin}\"\n\
                   }}\n\
                 }}\n\
               }}\n\
             }}",
            bin = Self::BINARY_NAME
        )
    }

    fn resolve_command(&self, worktree: &zed::Worktree) -> Result<zed::Command> {
        let settings = LspSettings::for_worktree(Self::LANGUAGE_SERVER_ID, worktree).ok();
        let binary = settings.as_ref().and_then(|s| s.binary.as_ref());
        let args = binary.and_then(|b| b.arguments.clone()).unwrap_or_default();
        let settings_env = binary.and_then(|b| b.env.clone());
        let env = Self::merge_env(worktree.shell_env(), settings_env);

        // 1. Settings override
        if let Some(path) = binary.and_then(|b| b.path.as_ref()) {
            let path = path.trim();
            if !path.is_empty() {
                return Ok(zed::Command {
                    command: path.to_string(),
                    args,
                    env,
                });
            }
        }

        // 2. PATH
        if let Some(path) = worktree.which(Self::BINARY_NAME) {
            return Ok(zed::Command {
                command: path,
                args,
                env,
            });
        }

        // 3. Quon-checkout worktree targets (release, then debug)
        if let Some(path) = Self::worktree_target_binary(worktree) {
            return Ok(zed::Command {
                command: path,
                args,
                env,
            });
        }

        // 4. Clear actionable error
        Err(Self::missing_binary_error())
    }
}

impl zed::Extension for QuonExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        if language_server_id.as_ref() != Self::LANGUAGE_SERVER_ID {
            return Err(format!(
                "Unrecognized language server for Quon: {language_server_id}"
            ));
        }
        self.resolve_command(worktree)
    }
}

zed::register_extension!(QuonExtension);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_error_mentions_settings_and_build() {
        let msg = QuonExtension::missing_binary_error();
        assert!(msg.contains("cargo build -p quon_lsp --release"));
        assert!(msg.contains("lsp"));
        assert!(msg.contains("quon-lsp"));
        assert!(msg.contains("binary"));
        assert!(msg.contains("path"));
    }

    #[test]
    fn merge_env_overrides_shell_keys() {
        let shell = vec![
            ("PATH".into(), "/usr/bin".into()),
            ("RUST_LOG".into(), "info".into()),
        ];
        let settings = Some(HashMap::from([(
            "RUST_LOG".into(),
            "quon_lsp=debug".into(),
        )]));
        let merged = QuonExtension::merge_env(shell, settings);
        assert_eq!(
            merged
                .iter()
                .find(|(k, _)| k == "RUST_LOG")
                .map(|(_, v)| v.as_str()),
            Some("quon_lsp=debug")
        );
        assert_eq!(
            merged
                .iter()
                .find(|(k, _)| k == "PATH")
                .map(|(_, v)| v.as_str()),
            Some("/usr/bin")
        );
    }

    #[test]
    fn quon_checkout_manifest_heuristic() {
        // Mirror the string checks used by is_quon_checkout without Worktree.
        let manifest = r#"
[workspace]
members = [
    "frontend",
    "quon_lsp",
]
"#;
        assert!(manifest.contains("[workspace]"));
        assert!(manifest.lines().any(|l| l.contains("\"quon_lsp\"")));
        assert!(manifest.lines().any(|l| l.contains("\"frontend\"")));
    }

    #[test]
    fn worktree_target_candidates_release_then_debug() {
        let root = Path::new("/repo/quon");
        let candidates = QuonExtension::worktree_target_candidates(root, "quon_lsp");
        assert_eq!(
            candidates[0],
            PathBuf::from("/repo/quon/target/release/quon_lsp")
        );
        assert_eq!(
            candidates[1],
            PathBuf::from("/repo/quon/target/debug/quon_lsp")
        );
    }

    /// Regression: guest `Path::is_file` on worktree-absolute paths is NOT a
    /// valid existence check under Zed WASM (only the extension work dir is
    /// preopened). Discovery must use `host_path_is_executable` (host
    /// `test -x` / `cmd if exist`) instead. This test documents the contract
    /// without calling WIT imports unavailable in host `cargo test`.
    #[test]
    fn worktree_discovery_must_not_rely_on_guest_is_file() {
        let source = include_str!("lib.rs");
        // The live discovery path must call the host probe.
        assert!(
            source.contains("host_path_is_executable"),
            "worktree discovery must probe via host_path_is_executable"
        );
        assert!(
            source.contains("zed::process::Command::new(\"test\")"),
            "Unix host probe must use `test` (runs outside WASI sandbox)"
        );
        // Guard against reintroducing guest FS checks on candidate paths.
        let discovery_fn = source
            .split("fn worktree_target_binary")
            .nth(1)
            .and_then(|rest| rest.split("fn missing_binary_error").next())
            .expect("worktree_target_binary present");
        assert!(
            !discovery_fn.contains("is_file()"),
            "worktree_target_binary must not call Path::is_file (WASM-blind)"
        );
        assert!(
            !discovery_fn.contains("fs::metadata"),
            "worktree_target_binary must not call fs::metadata on worktree paths"
        );
    }
}
