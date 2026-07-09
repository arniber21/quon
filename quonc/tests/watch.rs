//! Watch-mode debounce and event filtering tests (issue #48).

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use notify::event::{CreateKind, ModifyKind};
use notify::{Event, EventKind};
use quonc::watch::{debounce_deadline, event_targets_paths, is_relevant_event};

#[test]
fn debounce_waits_for_quiet_window() {
    let (tx, rx) = mpsc::channel();
    tx.send(Ok(Event::new(EventKind::Modify(ModifyKind::Data(
        notify::event::DataChange::Any,
    )))))
    .unwrap();
    let start = std::time::Instant::now();
    debounce_deadline(&rx, 50).expect("debounce");
    assert!(start.elapsed() >= Duration::from_millis(45));
}

#[test]
fn relevant_modify_event_matches_source_path() {
    let dir = std::env::temp_dir();
    let file = dir.join(format!("quon-watch-{}.qn", std::process::id()));
    std::fs::write(&file, "fn main(): Q<Qubit> = run { return qreg(1).0 }").unwrap();

    let event = Event {
        kind: EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
        paths: vec![file.clone()],
        attrs: Default::default(),
    };
    assert!(is_relevant_event(&event));
    assert!(event_targets_paths(&event, std::slice::from_ref(&file)));

    let _ = std::fs::remove_file(&file);
}

#[test]
fn create_event_is_relevant() {
    let event = Event {
        kind: EventKind::Create(CreateKind::Any),
        paths: vec![PathBuf::from("x.qn")],
        attrs: Default::default(),
    };
    assert!(is_relevant_event(&event));
}
