// SPDX-License-Identifier: MIT

use super::*;

fn entry(location: Location) -> FileEntry {
    FileEntry {
        location,
        native_name: "fixture".into(),
        thumbnail_path: None,
        display_name: "fixture".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        mode: crate::model::MetadataValue::Unknown,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
        recent_uri: None,
    }
}

#[test]
fn prepared_selection_rejects_changed_targets() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let location = Location::local(directory.path().join("alpha.txt"));
    let selection = OpenWithSelection {
        locations: vec![location.clone()],
        files: vec![gio_file_for_location(&location)],
        content_types: vec!["text/plain".to_string()],
        recommended_apps: vec![],
        other_apps: vec![],
        default: None,
    };
    assert!(selection.entries_match_target(&[entry(location)]));
    assert!(!selection.entries_match_target(&[]));
    assert!(
        !selection
            .entries_match_target(&[entry(Location::local(directory.path().join("beta.txt")))])
    );
}

#[cfg(windows)]
#[test]
fn native_open_with_is_available_without_registered_applications() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::open_with::native_open_with_is_available_without_registered_applications",
        || {
            let directory = tempfile::tempdir().expect("fixture directory");
            let entries = vec![
                entry(Location::local(
                    directory.path().join("unknown.strata-unregistered"),
                )),
                entry(Location::local(directory.path().join("another.txt"))),
            ];
            let single = gtk::Button::new();
            let multiple = gtk::Button::new();
            let open = gtk::Button::new();
            single.set_sensitive(false);
            multiple.set_sensitive(false);
            let result = Rc::new(RefCell::new(None));
            prepare_open_with(
                entries.clone(),
                &single,
                &multiple,
                &open,
                &result,
                &Rc::new(Cell::new(1)),
                1,
            );
            assert!(single.is_sensitive());
            assert!(multiple.is_sensitive());
            assert!(
                result
                    .borrow()
                    .as_ref()
                    .expect("native selection ready")
                    .entries_match_target(&entries)
            );
        },
    );
}
