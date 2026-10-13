// SPDX-License-Identifier: MIT

use super::*;
use std::sync::{Arc, Mutex};

use crate::services::{
    ActionRunEvent, ActionRunRequest, ActionRunner, InvocationSource,
};

const MANIFEST: &str = r#"
schema_version = 1
id = "preview-task"
name = "Preview task"

[when]
extensions = ["txt"]

[run]
runtime = "python"
entrypoint = "main.py"
mode = "per-item"
"#;

#[test]
fn windows_keeps_saved_action_but_never_executes_a_process_tree() {
    let fixture = tempfile::tempdir().expect("fixture");
    let store = LocalActionStore::at(fixture.path().join("actions"));
    let request = ActionWriteRequest {
        definition: ActionDefinition::parse(MANIFEST).expect("valid manifest"),
        script: Some(ActionScript {
            file_name: "main.py".into(),
            contents: "import pathlib; pathlib.Path('executed').write_text('BAD')".into(),
        }),
    };
    store.create(&request).expect("persist action manifest");
    let catalog = store.load();
    assert!(catalog.failures().is_empty(), "{:?}", catalog.failures());
    let action = catalog.get("preview-task").expect("saved action visible");
    assert!(!action.is_available(), "Windows process-tree isolation is required");
    assert!(action.unavailable_reason().unwrap().contains("isolation"));
    assert!(store.read_script("preview-task").unwrap().is_some());

    let events: Arc<Mutex<Vec<ActionRunEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&events);
    let cancel = crate::adapters::LocalActionRunner::new().run(
        &ActionRunRequest {
            action: Rc::clone(action),
            inputs: vec![fixture.path().join("sample.txt")],
            parent: fixture.path().to_path_buf(),
            source: InvocationSource::Selection,
            position: None,
        },
        Arc::new(move |event| received.lock().unwrap().push(event)),
    );
    cancel();
    let events = events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(&events[0], ActionRunEvent::Failed(message) if message.contains("isolation")));
    assert!(!fixture.path().join("executed").exists());
}

#[test]
fn windows_refuses_reparse_action_roots_without_following_them() {
    let fixture = tempfile::tempdir().expect("fixture");
    let real = fixture.path().join("real");
    let linked = fixture.path().join("linked");
    fs::create_dir(&real).unwrap();
    if std::os::windows::fs::symlink_dir(&real, &linked).is_ok() {
        let store = LocalActionStore::at(linked);
        assert!(store.load().actions().is_empty());
        assert!(ensure_private_directory(&store.root).is_err());
    }
}
