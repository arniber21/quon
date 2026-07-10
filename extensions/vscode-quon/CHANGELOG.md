# Changelog

## 0.1.1

- Fix packaged `.vsix` omitting `vscode-languageclient` (`.vscodeignore` had `node_modules/**`), which left TextMate highlighting working while hover, diagnostics, and Quon commands failed to activate (`command 'quon.showServerStatus' not found`).
- Harden `activate()`: keep commands/formatter registered if `quon_lsp` fails to start; surface a clear error toast.
- Activate on Quon commands as well as `onLanguage:quon`.

## 0.1.0

- Initial Quon VS Code extension: TextMate grammar, `quon_lsp` client, `quonfmt` Format Document provider.
- Format-on-save default OFF (comment-stripping hazard).
- Apache-2.0 LICENSE.
