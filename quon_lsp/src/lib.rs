//! Quon language server — stdio LSP with incremental type-check diagnostics.
//!
//! Launch the binary directly after building:
//!
//! ```text
//! cargo build -p quon_lsp
//! target/debug/quon_lsp
//! ```
//!
//! Editor wiring (VS Code / Neovim / Cursor):
//!
//! ```json
//! {
//!   "languages": [{
//!     "fileExtensions": [".qn"],
//!     "languageId": "quon"
//!   }],
//!   "server": {
//!     "command": "/path/to/target/debug/quon_lsp",
//!     "transport": "stdio"
//!   }
//! }
//! ```
//!
//! Set `QUON_LOG=debug` (or `RUST_LOG=quon_lsp=debug`) for stderr tracing.
//! Optional `QUON_LSP_DEBOUNCE_MS` overrides the analysis debounce interval.

pub mod analysis;
pub mod diagnostics;
pub mod document;
pub mod server;
pub mod span;

pub use analysis::AnalysisScheduler;
pub use diagnostics::{check_to_lsp_diags, diagnostic_to_lsp};
pub use document::{Document, DocumentStore};
pub use server::QuonLanguageServer;
pub use span::LineIndex;
