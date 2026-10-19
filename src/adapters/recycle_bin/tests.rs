// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    adapters::LocalFileSource,
    services::{FileSource, RequestId},
};
use gtk::prelude::*;
use std::cell::RefCell;

fn wait(context: &glib::MainContext, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        while context.pending() {
            context.iteration(false);
        }
        assert!(Instant::now() < deadline, "native bin request timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn load(
    context: &glib::MainContext,
    source: &LocalFileSource,
    location: Location,
) -> Vec<FileEntry> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let _load = source.enumerate(
        DirectoryRequest {
            id: RequestId(9001),
            location,
            batch_size: 2,
            include_metadata: true,
            max_entries: 100_000,
            time_budget: Duration::from_secs(10),
        },
        Rc::new(move |event| captured.borrow_mut().push(event)),
    );
    wait(context, || {
        events.borrow().iter().any(|event| {
            matches!(
                event,
                DirectoryEvent::Finished { .. } | DirectoryEvent::Failed { .. }
            )
        })
    });
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, DirectoryEvent::Failed { .. })),
        "{:#?}",
        events.borrow()
    );
    let mut entries = Vec::new();
    for event in events.borrow_mut().drain(..) {
        if let DirectoryEvent::Batch { entries: batch, .. } = event {
            entries.extend(batch);
        }
    }
    entries
}

#[test]
fn native_bin_lists_original_names_browses_folders_and_resolves_restore_destinations() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .unwrap();
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ảnh # %.txt");
    let folder = dir.path().join("nested.folder");
    std::fs::write(&file, b"preview text").unwrap();
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("child.json"), b"{}").unwrap();
    let items = strata_win_shell::recycle(&[file.clone(), folder.clone()]).unwrap();
    let source = LocalFileSource::default();
    assert!(
        source
            .validate_location(&Location::uri("trash:///"))
            .is_ok()
    );
    let entries = load(&context, &source, Location::uri("trash:///"));
    let browser = crate::app::Browser::new(Rc::new(LocalFileSource::default()));
    let root = Location::uri("trash:///");
    browser.navigate_location(root.clone(), true);
    wait(&context, || {
        browser
            .column_snapshot(0)
            .is_some_and(|column| !column.loading)
    });
    assert_eq!(browser.active_location(), Some(root));
    assert!(browser.column_snapshot(0).unwrap().error.is_none());
    for item in &items {
        assert!(
            browser
                .with_column_entries(0, |entries| entries
                    .iter()
                    .any(|entry| { path(&entry.location).as_ref() == Some(&item.recycled) }))
                .unwrap()
        );
    }
    assert_eq!(browser.can_trash_at(0), Some(false));
    assert_eq!(browser.can_delete_at(0), Some(true));
    drop(browser);
    for item in &items {
        let entry = entries
            .iter()
            .find(|entry| path(&entry.location).as_ref() == Some(&item.recycled))
            .expect("listed native bin identity");
        assert_eq!(entry.native_name, item.original.file_name().unwrap());
        let exported = crate::adapters::gio_file_for_location(&entry.location);
        assert!(
            !exported.is_native(),
            "clipboard/export must retain the virtual identity"
        );
        assert_eq!(exported.uri().as_str(), entry.location.uri_value().unwrap());
        let readable = crate::adapters::read_file_for_location(&entry.location);
        assert_eq!(readable.path().as_deref(), Some(item.recycled.as_path()));
        assert_eq!(
            entry.thumbnail_path.as_deref(),
            Some(item.recycled.as_path())
        );
        let destinations = context.block_on(
            super::super::trash_restore::restore_destinations_for_locations(vec![(
                entry.location.clone(),
                entry.thumbnail_path.clone(),
            )]),
        );
        assert_eq!(destinations[0].as_ref().unwrap(), &item.original);
        if item.original == folder {
            let children = load(&context, &source, entry.location.clone());
            assert_eq!(children.len(), 1);
            assert_eq!(children[0].display_name, "child.json");
            assert_eq!(
                children[0].location.parent().as_ref(),
                Some(&entry.location)
            );
            assert!(!is_item(&children[0].location));
            assert_eq!(
                std::fs::read(path(&children[0].location).unwrap()).unwrap(),
                b"{}"
            );
        }
    }
    use crate::services::{
        OperationEvent, OperationProvider, OperationRequestId, RestoreRequest, RestoreSource,
        RestoreTrashItem,
    };
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let provider = crate::adapters::LocalOperationProvider::default();
    let selected = items
        .iter()
        .map(|item| RestoreTrashItem {
            entry: entries
                .iter()
                .find(|entry| path(&entry.location).as_ref() == Some(&item.recycled))
                .unwrap()
                .clone(),
            destination: item.original.clone(),
        })
        .collect();
    let _restore = provider.restore(
        RestoreRequest {
            id: OperationRequestId(9003),
            source: RestoreSource::TrashEntries(selected),
        },
        Rc::new(move |event| captured.borrow_mut().push(event)),
    );
    wait(&context, || {
        events.borrow().iter().any(|event| {
            matches!(
                event,
                OperationEvent::Restored { .. } | OperationEvent::Failed { .. }
            )
        })
    });
    assert!(events.borrow().iter().any(|event| matches!(event, OperationEvent::Restored { locations, .. } if locations.len() == 2)), "{:#?}", events.borrow());
    for item in items {
        assert!(!item.recycled.exists());
    }
    assert_eq!(std::fs::read(file).unwrap(), b"preview text");
}

#[test]
fn virtual_identities_preserve_windows_names_and_reject_traversal() {
    let file = Path::new(r"C:\$Recycle.Bin\SID\$Rabc # %.txt");
    let identity = location(file);
    assert_eq!(path(&identity).as_deref(), Some(file));
    assert!(is_item(&identity));
    let child = child_location(&identity, std::ffi::OsStr::new("ảnh.json"));
    assert_eq!(path(&child), Some(file.join("ảnh.json")));
    for name in ["..", ".", r"C:\live.txt", r"a\b", "a/b"] {
        assert!(path(&child_location(&identity, std::ffi::OsStr::new(name))).is_none());
    }
    for uri in [
        "trash:///windows-",
        "trash:///windows-zzzz",
        "trash:///windows-0000",
        "trash:///windows-002e002e",
    ] {
        assert!(path(&Location::uri(uri)).is_none());
    }
}

#[test]
fn dropping_native_bin_load_prevents_stale_callbacks() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .unwrap();
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    let called = Rc::new(std::cell::Cell::new(false));
    let captured = called.clone();
    let load = LocalFileSource::default().enumerate(
        DirectoryRequest {
            id: RequestId(9002),
            location: Location::uri("trash:///"),
            batch_size: 1,
            include_metadata: false,
            max_entries: 1,
            time_budget: Duration::from_secs(1),
        },
        Rc::new(move |_| captured.set(true)),
    );
    drop(load);
    while context.pending() {
        context.iteration(false);
    }
    assert!(!called.get());
}
