// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn smart_search_keeps_filename_results_when_content_changes_and_preview_starts_on_selection() {
    crate::test_support::gtk_test(
        "ui::search::smart::tests::smart_search_keeps_filename_results_when_content_changes_and_preview_starts_on_selection",
        || {
            let panel = Panel::new(Rc::new(|_| {}));
            panel.query("invoice");
            let filename = SearchItem::for_test("/fixture/invoice.txt".into(), false);
            panel.names(vec![filename.clone()]);
            assert_eq!(panel.store.n_items(), 1);
            assert!(panel.drawer.borrow().is_none());
            assert!(panel.selected().is_none());
            panel.move_selection(1);
            assert_eq!(panel.selected().expect("selection").path, filename.path);
            panel.refresh_content();
            assert_eq!(panel.store.n_items(), 1);
            assert_eq!(
                panel.selected().expect("retained selection").path,
                filename.path
            );
            panel.cancel();
            assert!(!panel.preview_slot.is_visible());
            panel.names(vec![
                SearchItem::for_test("/docs/invoice.txt".into(), false),
                SearchItem::for_test("/elsewhere/invoice.txt".into(), false),
                SearchItem::for_test("/docs/source.rs".into(), false),
                SearchItem::for_test("/docs/folder.rs".into(), true),
            ]);
            panel.scope.replace(Some("/docs".into()));
            panel.kind.set(1);
            panel.render();
            assert_eq!(
                panel
                    .rows
                    .borrow()
                    .iter()
                    .map(|row| row.path.clone())
                    .collect::<Vec<_>>(),
                vec![std::path::PathBuf::from("/docs/invoice.txt")]
            );
            panel.kind.set(5);
            panel.render();
            assert_eq!(
                panel
                    .rows
                    .borrow()
                    .iter()
                    .map(|row| row.path.clone())
                    .collect::<Vec<_>>(),
                vec![std::path::PathBuf::from("/docs/source.rs")]
            );
        },
    );
}
