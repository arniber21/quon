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
    let tower_lsp::lsp_types::SemanticTokensResult::Tokens(t) = tokens else {
        panic!("expected full tokens");
    };
    assert!(
        t.data
            .iter()
            .any(|tok| tok.token_type == 0 && tok.length == 2),
        "expected `fn` keyword token (type 0), got {:?}",
        t.data
    );
}

#[test]
fn gate_identifier_gets_function_token_type() {
    let src = "fn f(): Circuit<1, 1, 1, Clifford> = circuit { H @0 }\n";
    let result = analyze(src);
    let tokens = quon_lsp::intel::semantic_tokens_full(
        &result.intelligence,
        tower_lsp::lsp_types::Position {
            line: 0,
            character: 0,
        },
    )
    .expect("tokens");
    let tower_lsp::lsp_types::SemanticTokensResult::Tokens(t) = tokens else {
        panic!("expected full tokens");
    };
    assert!(
        t.data
            .iter()
            .any(|tok| tok.token_type == 2 && tok.length == 1),
        "expected gate `H` as function token (type 2), got {:?}",
        t.data
    );
}
