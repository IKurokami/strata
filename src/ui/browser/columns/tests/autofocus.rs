// SPDX-License-Identifier: MIT

use super::*;

fn fixture() -> Columns {
    let fixture = Columns::new(0, 1200);
    let preferences = PreferenceManager::shared();
    preferences.set_columns_mirror_selection(true);
    preferences.set_sort_preferences(crate::model::ViewPreferences::default());
    fixture
}

fn first_name(view: &BrowserView) -> Option<(usize, String)> {
    view.browser()
        .focused_item()
        .map(|(depth, _, entry)| (depth, entry.display_name))
}

#[test]
fn branching_folder_keeps_focus_while_opening_only_its_first_child() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::autofocus::branching_folder_keeps_focus_while_opening_only_its_first_child",
        || {
            let fixture = fixture();
            let root = fixture._root.path().join("downloads");
            std::fs::create_dir_all(root.join("a-first/nested/deeper")).unwrap();
            std::fs::write(root.join("z.txt"), b"z").unwrap();
            fixture.view.browser().navigate(Location::local(&root));
            pump_until(|| column_loaded(&fixture.view, 0), "origin load");
            assert_eq!(first_name(&fixture.view), Some((0, "a-first".into())));
            pump_until(|| column_loaded(&fixture.view, 1), "first child");
            pump_for(Duration::from_millis(150));
            assert_eq!(first_name(&fixture.view), Some((0, "a-first".into())));
            assert!(fixture.view.browser().selection_is_load_cursor());
            assert!(fixture.view.browser().column_snapshot(2).is_none());
            assert!(fixture.view.browser().selected_positions(1).is_empty());
            assert_eq!(fixture.view.state.focused_column_depth(), Some(0));
            let first = fixture.view.browser().focused_item().unwrap().1;
            fixture.view.browser().activate(0, first);
            assert_eq!(first_name(&fixture.view), Some((1, "nested".into())));
            assert!(fixture.view.browser().selection_is_load_cursor());

            // Refreshing is not a new navigation and must retain a later explicit cursor.
            let z = (0..2)
                .find(|&position| {
                    fixture
                        .view
                        .browser()
                        .entry_at(0, position)
                        .is_some_and(|entry| entry.display_name == "z.txt")
                })
                .unwrap();
            fixture.view.browser().select(0, z);
            fixture.view.browser().reload_active();
            pump_until(|| column_loaded(&fixture.view, 0), "refresh");
            pump_for(Duration::from_millis(100));
            assert_eq!(first_name(&fixture.view), Some((0, "z.txt".into())));
        },
    );
}

#[test]
fn single_folder_chains_stop_at_a_branch_or_the_load_budget() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::autofocus::single_folder_chains_stop_at_a_branch_or_the_load_budget",
        || {
            let fixture = fixture();
            for (length, expected_depth) in [(2, 2), (8, 4)] {
                let root = fixture._root.path().join(format!("chain-{length}"));
                let mut tail = root.clone();
                for _ in 0..length {
                    tail = tail.join("only");
                }
                std::fs::create_dir_all(tail.join("a-child")).unwrap();
                std::fs::write(tail.join("z.txt"), b"z").unwrap();
                fixture.view.browser().navigate(Location::local(&root));
                pump_until(
                    || first_name(&fixture.view).is_some_and(|(depth, _)| depth == expected_depth),
                    "bounded chain cursor",
                );
                let last_loaded = if length == 2 { 3 } else { 4 };
                pump_until(
                    || column_loaded(&fixture.view, last_loaded),
                    "bounded child load",
                );
                pump_for(Duration::from_millis(150));
                assert_eq!(fixture.view.browser().active_depth(), Some(expected_depth));
                assert_eq!(
                    fixture.view.state.focused_column_depth(),
                    Some(expected_depth)
                );
                assert!(
                    fixture
                        .view
                        .browser()
                        .column_snapshot(last_loaded + 1)
                        .is_none()
                );
                assert!(fixture.view.browser().selection_is_load_cursor());
            }
        },
    );
}

#[test]
fn reveal_targets_and_new_navigation_supersede_automatic_focus() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::autofocus::reveal_targets_and_new_navigation_supersede_automatic_focus",
        || {
            let fixture = fixture();
            let root = fixture._root.path().join("reveal");
            std::fs::create_dir_all(root.join("a-folder/nested")).unwrap();
            std::fs::write(root.join("z.txt"), b"z").unwrap();
            fixture.view.browser().reveal_locations(
                Location::local(&root),
                vec![Location::local(root.join("z.txt"))],
            );
            pump_until(|| column_loaded(&fixture.view, 0), "reveal directory");
            pump_for(Duration::from_millis(100));
            assert_eq!(first_name(&fixture.view), Some((0, "z.txt".into())));
            assert!(fixture.view.browser().column_snapshot(1).is_none());

            fixture.view.browser().navigate(Location::local(&root));
            let other = fixture._root.path().join("other");
            std::fs::create_dir_all(&other).unwrap();
            std::fs::write(other.join("chosen.txt"), b"chosen").unwrap();
            fixture.view.browser().navigate(Location::local(&other));
            pump_until(|| column_loaded(&fixture.view, 0), "replacement directory");
            pump_for(Duration::from_millis(100));
            assert_eq!(first_name(&fixture.view), Some((0, "chosen.txt".into())));
            assert!(fixture.view.browser().column_snapshot(1).is_none());
        },
    );
}
