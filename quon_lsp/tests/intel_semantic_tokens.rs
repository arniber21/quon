mod support;

use frontend::analyze;
use tower_lsp::lsp_types::{Position, Range};

const MOD_DEFINITION: u32 = 1 << 0;
const MOD_READONLY: u32 = 1 << 1;

#[test]
fn tokens_emitted_for_program() {
    let src = "fn f(): Int = 1\n";
    let result = analyze(src);
    let tokens = quon_lsp::intel::semantic_tokens_full(
        &result.intelligence,
        Position {
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
        Position {
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
        Position {
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

#[test]
fn legend_includes_definition_and_readonly_modifiers() {
    let legend = quon_lsp::intel::semantic_tokens_legend();
    assert_eq!(
        legend.token_modifiers,
        vec!["definition".into(), "readonly".into()]
    );
}

#[test]
fn function_def_gets_definition_and_readonly_modifiers() {
    let src = "fn f(): Int = 1\n";
    let result = analyze(src);
    let tokens = quon_lsp::intel::semantic_tokens_full(
        &result.intelligence,
        Position {
            line: 0,
            character: 0,
        },
    )
    .expect("tokens");
    let tower_lsp::lsp_types::SemanticTokensResult::Tokens(t) = tokens else {
        panic!("expected full tokens");
    };
    // `f` is a 1-char function name at its definition site.
    let f_tok = t
        .data
        .iter()
        .find(|tok| tok.token_type == 2 && tok.length == 1)
        .expect("function name token");
    assert_eq!(
        f_tok.token_modifiers_bitset & MOD_DEFINITION,
        MOD_DEFINITION,
        "expected definition modifier on `f`"
    );
    assert_eq!(
        f_tok.token_modifiers_bitset & MOD_READONLY,
        MOD_READONLY,
        "expected readonly on reusable function `f`"
    );
}

#[test]
fn linear_param_omits_readonly_modifier() {
    let src = "fn use_q(q: Qubit): Qubit = q\n";
    let result = analyze(src);
    let tokens = quon_lsp::intel::semantic_tokens_full(
        &result.intelligence,
        Position {
            line: 0,
            character: 0,
        },
    )
    .expect("tokens");
    let tower_lsp::lsp_types::SemanticTokensResult::Tokens(t) = tokens else {
        panic!("expected full tokens");
    };
    // Parameter `q` at definition — length 1, parameter type index 4.
    // Parameter `q` at definition — length 1, parameter type index 4.
    let q_def = t
        .data
        .iter()
        .find(|tok| {
            tok.token_type == 4
                && tok.length == 1
                && (tok.token_modifiers_bitset & MOD_DEFINITION) != 0
        })
        .expect("parameter `q` definition token");
    assert_eq!(
        q_def.token_modifiers_bitset & MOD_READONLY,
        0,
        "linear `q: Qubit` must not get readonly; got {:?}",
        q_def
    );
}

#[test]
fn range_request_filters_to_requested_span() {
    let src = "fn a(): Int = 1\nfn b(): Int = 2\n";
    let result = analyze(src);
    let full = quon_lsp::intel::semantic_tokens_full(
        &result.intelligence,
        Position {
            line: 0,
            character: 0,
        },
    )
    .expect("full");
    let tower_lsp::lsp_types::SemanticTokensResult::Tokens(full_toks) = full else {
        panic!("expected full tokens");
    };

    let range = Range {
        start: Position {
            line: 1,
            character: 0,
        },
        end: Position {
            line: 2,
            character: 0,
        },
    };
    let ranged =
        quon_lsp::intel::semantic_tokens_range(&result.intelligence, range).expect("range");
    let tower_lsp::lsp_types::SemanticTokensRangeResult::Tokens(range_toks) = ranged else {
        panic!("expected range tokens");
    };

    assert!(
        !range_toks.data.is_empty(),
        "line 1 should still emit tokens"
    );
    assert!(
        range_toks.data.len() < full_toks.data.len(),
        "range should emit fewer tokens than full (range={}, full={})",
        range_toks.data.len(),
        full_toks.data.len()
    );
}
