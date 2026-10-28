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
        },
    );
}
