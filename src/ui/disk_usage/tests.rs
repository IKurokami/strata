// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn canvas_exposes_real_accessible_items_and_replaces_the_old_accessibility_tree() {
    crate::test_support::gtk_test(
        "ui::disk_usage::tests::canvas_exposes_real_accessible_items_and_replaces_the_old_accessibility_tree",
        || {
            let canvas = accessibility::Canvas::new();
            canvas.set_items(&["Folder A · 20 B".into(), "file.txt · 7 B".into()]);
            let first = canvas.first_accessible_child().expect("folder accessible");
            assert_eq!(first.accessible_role(), gtk::AccessibleRole::ListItem);
            let second = first.next_accessible_sibling().expect("file accessible");
            assert!(second.next_accessible_sibling().is_none());
            canvas.set_selected(Some(1));
            canvas.set_items(&["Replacement · 3 B".into()]);
            assert_ne!(canvas.first_accessible_child().unwrap(), first);
            assert!(
                first.accessible_parent().is_none(),
                "retired accessible cannot retain a stale parent"
            );
            canvas.set_items(&[]);
            assert!(canvas.first_accessible_child().is_none());
        },
    );
}
