// SPDX-License-Identifier: MIT

use super::pointer_focus::PreviewCalls;
use super::*;
use crate::ui::preview::PreviewDrawer;

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
            assert_eq!(fixture.view.browser().selected_positions(1), vec![0]);
            assert_eq!(
                fixture
                    .view
                    .browser()
                    .cursor_entry(1)
                    .expect("child cursor")
                    .display_name,
                "nested"
            );
            assert_eq!(fixture.view.state.focused_column_depth(), Some(0));
            let first = fixture.view.browser().focused_item().unwrap().1;
            fixture.view.browser().enter_focused_directory();
            pump_until(
                || {
                    column_loaded(&fixture.view, 2)
                        && fixture.view.browser().cursor_entry(2).is_some()
                },
                "keyboard folder mirror",
            );
            assert_eq!(first_name(&fixture.view), Some((1, "nested".into())));
            assert_eq!(
                fixture
                    .view
                    .browser()
                    .cursor_entry(2)
                    .expect("mirrored folder cursor")
                    .display_name,
                "deeper"
            );
            assert!(fixture.view.browser().location_at(3).is_none());
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

#[test]
fn selecting_another_folder_seeds_its_files_without_moving_focus_or_reopening_preview() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::autofocus::selecting_another_folder_seeds_its_files_without_moving_focus_or_reopening_preview",
        || {
            let fixture = fixture();
            PreferenceManager::shared().set_single_click_previews(true);
            PreferenceManager::shared().set_click_activation(
                BrowserMode::Columns,
                crate::ui::browser_modes::ClickActivation::default_for(BrowserMode::Columns),
            );
            let root = fixture._root.path().join("downloads");
            let folder = "38833FF26BA1D.UnigramPreview_g9c9v27vpyspw!App";
            std::fs::create_dir_all(root.join("000-initial")).expect("initial folder");
            std::fs::write(root.join("000-initial/old.txt"), b"old").expect("old file");
            std::fs::create_dir_all(root.join(folder)).expect("target folder");
            for name in ["0001.jpg", "0002.jpg", "data.txt"] {
                std::fs::write(root.join(folder).join(name), b"fixture").expect("target file");
            }
            let app = fixture.view.browser();
            app.navigate(Location::local(&root));
            pump_until(
                || column_loaded(&fixture.view, 1) && app.cursor_entry(1).is_some(),
                "initial child cursor",
            );
            let provider = Rc::new(PreviewCalls::default());
            let preview = PreviewDrawer::new(provider.clone(), false);
            preview.observe_browser(&app);
            preview.show(app.cursor_entry(1).expect("old file"), Some(1));
            let calls = provider.0.get();
            let position = (0..2)
                .find(|&pos| {
                    app.entry_at(0, pos)
                        .is_some_and(|entry| entry.display_name == folder)
                })
                .expect("target position");
            app.select(0, position);
            app.show_child(0, Location::local(root.join(folder)));
            pump_until(
                || column_loaded(&fixture.view, 1) && app.cursor_entry(1).is_some(),
                "new child cursor",
            );
            pump_for(Duration::from_millis(250));
            assert_eq!(first_name(&fixture.view), Some((0, folder.into())));
            assert_eq!(
                app.cursor_entry(1).expect("first child item").display_name,
                "0001.jpg"
            );
            assert_eq!(app.selected_positions(1), vec![0]);
            assert_eq!(fixture.view.state.focused_column_depth(), Some(0));
            assert!(!preview.is_open(), "load default reopened preview");
            assert_eq!(provider.0.get(), calls);
            assert!(app.location_at(2).is_none());
            app.enter_focused_directory();
            pump_for(Duration::from_millis(400));
            assert_eq!(first_name(&fixture.view), Some((1, "0001.jpg".into())));
            assert!(
                !preview.is_open(),
                "entering a folder previewed its automatic first file"
            );
            assert_eq!(provider.0.get(), calls);
            let column = fixture.view.state.columns.borrow()[1].clone();
            let (row, _) = column.shown_row_at(0).expect("visible first item");
            let controllers = row.observe_controllers();
            let click = (0..controllers.n_items())
                .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
                .find(|click| {
                    click.button() == 1
                        && click.propagation_phase() == gtk::PropagationPhase::Capture
                })
                .expect("primary row gesture");
            click.emit_by_name::<()>("pressed", &[&1i32, &1.0f64, &1.0f64]);
            click.emit_by_name::<()>("released", &[&1i32, &1.0f64, &1.0f64]);
            pump_until(|| preview.is_open(), "explicit image selection preview");
            assert_eq!(provider.0.get(), calls + 1);
        },
    );
}

#[test]
fn navigating_to_a_file_first_folder_does_not_reopen_preview_for_its_load_cursor() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::autofocus::navigating_to_a_file_first_folder_does_not_reopen_preview_for_its_load_cursor",
        || {
            let fixture = fixture();
            let app = fixture.view.browser();
            let provider = Rc::new(PreviewCalls::default());
            let preview = PreviewDrawer::new(provider.clone(), false);
            preview.observe_browser(&app);
            for (mode, name) in [
                (BrowserMode::Columns, "columns"),
                (BrowserMode::List, "list"),
                (BrowserMode::Icons, "icons"),
            ] {
                let root = fixture._root.path().join(name);
                std::fs::create_dir_all(&root).expect("text folder");
                std::fs::write(root.join("first.txt"), b"first").expect("first file");
                let location = Location::local(&root);
                PreferenceManager::shared().set_folder_mode(&location, mode);
                let mut old = crate::test_support::operations::entry(Location::local(
                    fixture._root.path().join("old.txt"),
                ));
                old.native_name = "old.txt".into();
                preview.show(old, Some(0));
                let calls = provider.0.get();
                app.navigate(location);
                pump_until(|| column_loaded(&fixture.view, 0), "file-first folder");
                pump_for(Duration::from_millis(250));
                assert_eq!(first_name(&fixture.view), Some((0, "first.txt".into())));
                assert!(!preview.is_open());
                assert_eq!(provider.0.get(), calls);
                app.select(0, 0);
                pump_until(|| preview.is_open(), "intentional file preview");
                assert_eq!(provider.0.get(), calls + 1);
            }
        },
    );
}

#[test]
fn file_command_sources_stay_in_the_focused_parent_with_a_filtered_child() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::autofocus::file_command_sources_stay_in_the_focused_parent_with_a_filtered_child",
        || {
            let fixture = fixture();
            let root = fixture._root.path().join("command-source");
            let folder = root.join("a-folder");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("inside.txt"), b"child").unwrap();
            std::fs::write(root.join("z.txt"), b"parent").unwrap();
            let empty = root.join("empty-folder");
            std::fs::create_dir_all(&empty).unwrap();
            let app = fixture.view.browser();
            app.navigate(Location::local(&root));
            pump_until(
                || column_loaded(&fixture.view, 1) && app.cursor_entry(1).is_some(),
                "passive child selection",
            );
            let child = fixture.view.state.columns.borrow()[1].clone();
            for query in ["", "inside"] {
                child.filter_entry.set_text(query);
                pump_for(Duration::from_millis(150));
                fixture.view.state.focus_column_background(0);
                fixture
                    .view
                    .record_pointer_hover((20.0, query.len() as f64), Some(1));
                assert_eq!(fixture.view.state.focused_column_depth(), Some(0));
                let expected = Location::local(&folder);
                assert_eq!(
                    fixture
                        .view
                        .clipboard_targets()
                        .iter()
                        .map(|e| e.location.clone())
                        .collect::<Vec<_>>(),
                    vec![expected.clone()]
                );
                assert_eq!(
                    fixture
                        .view
                        .command_targets()
                        .iter()
                        .map(|e| e.location.clone())
                        .collect::<Vec<_>>(),
                    vec![expected]
                );
                assert!(fixture.view.selected_search_results().is_none());
                assert_eq!(app.active_depth(), Some(0));
                assert!(child.focus_surface());
                pump_for(Duration::from_millis(30));
                assert_eq!(
                    fixture
                        .view
                        .clipboard_targets()
                        .iter()
                        .map(|e| e.location.clone())
                        .collect::<Vec<_>>(),
                    vec![Location::local(folder.join("inside.txt"))]
                );
                assert_eq!(
                    fixture.view.selected_search_results().is_some(),
                    !query.is_empty()
                );
            }
            app.descend(0, Location::local(&empty));
            pump_until(
                || {
                    app.column_snapshot(1).is_some_and(|column| {
                        !column.loading && column.location == Location::local(&empty)
                    })
                },
                "empty child column",
            );
            fixture.view.state.focus_column_background(1);
            assert_eq!(
                fixture
                    .view
                    .clipboard_targets()
                    .iter()
                    .map(|entry| entry.location.clone())
                    .collect::<Vec<_>>(),
                vec![Location::local(&empty)]
            );
        },
    );
}
