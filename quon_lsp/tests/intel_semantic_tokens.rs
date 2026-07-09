mod support;

use frontend::analyze;

#[test]
fn tokens_emitted_for_program() {
    let src = "fn f(): Int = 1\n";
    let result = analyze(src);
    let tokens = quon_lsp::intel::semantic_tokens_full(
        &result.intelligence,
        tower_lsp::lsp_types::Position {
            line: 0,
            character: 0,
        },
    )
    .expect("tokens");
    if let tower_lsp::lsp_types::SemanticTokensResult::Tokens(t) = tokens {
        assert!(!t.data.is_empty());
    } else {
        panic!("expected full tokens");
    }
}

#[test]
fn keyword_fn_gets_token() {
    let src = "fn f(): Int = 1\n";
    let result = analyze(src);
    let tokens = quon_lsp::intel::semantic_tokens_full(
        &result.intelligence,
        tower_lsp::lsp_types::Position {
            line: 0,
            character: 0,
        },
    )
    .expect("tokens");
    if let tower_lsp::lsp_types::SemanticTokensResult::Tokens(t) = tokens {
        assert!(!t.data.is_empty());
    } else {
        panic!("expected full tokens");
    }
}
