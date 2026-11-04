// SPDX-License-Identifier: MIT

use super::*;
use crate::model::{EntryKind, Location, MetadataValue};

fn entry(kind: EntryKind, location: Location) -> FileEntry {
    FileEntry {
        location,
        thumbnail_path: None,
        native_name: "source.txt".into(),
        display_name: "source.txt".into(),
        kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn menu_requires_exactly_two_regular_native_files() {
    let file = entry(EntryKind::File, Location::local("source.txt"));
    for count in 0..=3 {
        assert_eq!(supports(&vec![file.clone(); count]), count == 2);
    }
    for kind in [
        EntryKind::Directory,
        EntryKind::DirectorySymbolicLink,
        EntryKind::FileSymbolicLink,
        EntryKind::SymbolicLink,
        EntryKind::Other,
    ] {
        assert!(!supports(&[
            file.clone(),
            entry(kind, Location::local("other"))
        ]));
    }
    assert!(!supports(&[
        file,
        entry(EntryKind::File, Location::uri("sftp://host/source.txt"))
    ]));
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(
            std::time::Instant::now() < deadline,
            "comparison did not settle"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn result_view(overlay: &gtk::Overlay) -> (gtk::Box, gtk::TextView) {
    let mut layer = overlay.first_child();
    let layer = loop {
        let current = layer.expect("comparison modal");
        if current.has_css_class("app-modal-layer") {
            break current.downcast::<gtk::Box>().unwrap();
        }
        layer = current.next_sibling();
    };
    let mut pending = vec![layer.clone().upcast::<gtk::Widget>()];
    while let Some(widget) = pending.pop() {
        if let Ok(text) = widget.clone().downcast::<gtk::TextView>() {
            return (layer, text);
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            pending.push(widget);
        }
    }
    panic!("comparison output missing");
}

#[test]
fn closed_comparison_cannot_publish_into_a_new_pair_and_navigation_dismisses_it() {
    crate::test_support::gtk_test(
        "ui::browser::compare::tests::closed_comparison_cannot_publish_into_a_new_pair_and_navigation_dismisses_it",
        || {
            use crate::ui::{
                browser::{BrowserView, PeekBehavior},
                preferences::PreferenceManager,
            };
            PreferenceManager::seed_saved_preferences_for_test();
            let dir = tempfile::tempdir().expect("fixture");
            let left = dir.path().join("left.txt");
            let old_right = dir.path().join("old.txt");
            let new_right = dir.path().join("new.txt");
            for (path, content) in [
                (&left, "before\n"),
                (&old_right, "old-pair\n"),
                (&new_right, "new-pair\n"),
            ] {
                std::fs::write(path, content).expect("source");
            }
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let host = gtk::Overlay::new();
            host.set_child(Some(&view.widget()));
            let window = gtk::Window::builder().child(&host).build();
            window.present();
            wait_until(|| host.is_mapped());
            let pair = |right: &std::path::Path| {
                vec![
                    entry(EntryKind::File, Location::local(&left)),
                    entry(EntryKind::File, Location::local(right)),
                ]
            };
            show(&view.state, pair(&old_right));
            let (old_layer, old_text) = result_view(&host);
            assert!(!old_text.is_editable());
            modal::dismiss_modal_layer(&old_layer, &host, None);
            wait_until(|| {
                text_compare::ComparisonPermit::reserve().is_some() && !old_layer.is_mapped()
            });
            assert!(
                old_text
                    .buffer()
                    .text(
                        &old_text.buffer().start_iter(),
                        &old_text.buffer().end_iter(),
                        true
                    )
                    .is_empty()
            );
            show(&view.state, pair(&new_right));
            let (layer, text) = result_view(&host);
            wait_until(|| {
                text.buffer()
                    .text(&text.buffer().start_iter(), &text.buffer().end_iter(), true)
                    .contains("+new-pair\n")
            });
            assert!(!text.is_editable());
            let result =
                text.buffer()
                    .text(&text.buffer().start_iter(), &text.buffer().end_iter(), true);
            assert!(!result.contains("old-pair"));
            view.browser().bump_navigation_generation();
            wait_until(|| !layer.is_mapped());
            assert_eq!(std::fs::read_to_string(&left).unwrap(), "before\n");
            assert_eq!(std::fs::read_to_string(&new_right).unwrap(), "new-pair\n");
            PreferenceManager::shared().release_bindings_within(&view.widget());
            view.browser().clear_observer();
            window.destroy();
        },
    );
}
