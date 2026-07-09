use quon_lsp::document::{DocumentError, DocumentStore};
use tower_lsp::lsp_types::{Position, Range, TextDocumentContentChangeEvent, Url};

#[test]
fn apply_incremental_change_replaces_range() {
    let uri: Url = "file:///test.qn".parse().expect("valid url");
    let mut store = DocumentStore::default();
    store.open(uri.clone(), "hello world".into(), 1);

    let range = Range {
        start: Position {
            line: 0,
            character: 6,
        },
        end: Position {
            line: 0,
            character: 11,
        },
    };
    store
        .apply_changes(
            &uri,
            Some(2),
            &[TextDocumentContentChangeEvent {
                range: Some(range),
                range_length: None,
                text: "quon".into(),
            }],
        )
        .expect("apply changes");
    let doc = store.get(&uri).expect("open");
    assert_eq!(doc.text, "hello quon");
    assert_eq!(doc.version, 2);
}

#[test]
fn apply_full_sync_replaces_document() {
    let uri: Url = "file:///test.qn".parse().expect("valid url");
    let mut store = DocumentStore::default();
    store.open(uri.clone(), "old".into(), 1);
    store
        .apply_changes(
            &uri,
            Some(2),
            &[TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "new".into(),
            }],
        )
        .expect("apply changes");
    let doc = store.get(&uri).expect("open");
    assert_eq!(doc.text, "new");
}

#[test]
fn apply_multiple_changes_in_sequence() {
    let uri: Url = "file:///test.qn".parse().expect("valid url");
    let mut store = DocumentStore::default();
    store.open(uri.clone(), "abc".into(), 1);
    store
        .apply_changes(
            &uri,
            Some(2),
            &[
                TextDocumentContentChangeEvent {
                    range: Some(Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 1,
                        },
                    }),
                    range_length: None,
                    text: "x".into(),
                },
                TextDocumentContentChangeEvent {
                    range: Some(Range {
                        start: Position {
                            line: 0,
                            character: 3,
                        },
                        end: Position {
                            line: 0,
                            character: 3,
                        },
                    }),
                    range_length: None,
                    text: "z".into(),
                },
            ],
        )
        .expect("apply changes");
    let doc = store.get(&uri).expect("open");
    assert_eq!(doc.text, "xbcz");
}

#[test]
fn unknown_uri_did_change_is_noop() {
    let uri: Url = "file:///missing.qn".parse().expect("valid url");
    let mut store = DocumentStore::default();
    assert_eq!(
        store.apply_changes(
            &uri,
            Some(1),
            &[TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "x".into(),
            }],
        ),
        Err(DocumentError::NotOpen(uri.clone()))
    );
}

#[test]
fn invalid_edit_is_rejected_without_mutation() {
    let uri: Url = "file:///test.qn".parse().expect("valid url");
    let mut store = DocumentStore::default();
    store.open(uri.clone(), "hello".into(), 1);

    let bad_range = Range {
        start: Position {
            line: 99,
            character: 0,
        },
        end: Position {
            line: 99,
            character: 1,
        },
    };
    assert_eq!(
        store.apply_changes(
            &uri,
            Some(2),
            &[TextDocumentContentChangeEvent {
                range: Some(bad_range),
                range_length: None,
                text: "x".into(),
            }],
        ),
        Err(DocumentError::InvalidEdit(uri.clone()))
    );
    let doc = store.get(&uri).expect("still open");
    assert_eq!(doc.text, "hello");
    assert_eq!(doc.version, 1);
}
