// SPDX-License-Identifier: MIT

use super::*;
use std::rc::Rc;

#[test]
fn result_replacements_keep_selected_paths_and_publish_consistent_rows() {
    crate::test_support::gtk_test(
        "ui::browser::columns::search::tests::result_replacements_keep_selected_paths_and_publish_consistent_rows",
        || {
            let model = gtk::StringList::new(&[]);
            let results = Rc::new(RefCell::new(Vec::<SearchItem>::new()));
            let selection = gtk::MultiSelection::new(Some(model.clone()));
            let syncing = Cell::new(false);
            let observed = results.clone();
            model.connect_items_changed(move |model, _, _, _| {
                let results = observed.borrow();
                assert_eq!(model.n_items() as usize, results.len());
                for (position, item) in results.iter().enumerate() {
                    assert_eq!(
                        model.string(position as u32).as_deref(),
                        Some(item.name.as_str())
                    );
                }
            });
            let mut items: Vec<_> = ["a.txt", "b.txt", "c.txt"]
                .into_iter()
                .map(|name| {
                    SearchItem::for_test(std::path::PathBuf::from("/search").join(name), false)
                })
                .collect();
            assert!(!update_results(
                &model,
                &results,
                &selection,
                &syncing,
                items.clone()
            ));
            selection.select_item(1, true);
            items.swap(0, 1);
            items[0].name = "updated name".into();
            assert!(!update_results(
                &model,
                &results,
                &selection,
                &syncing,
                items.clone()
            ));
            assert!(selection.is_selected(0));
            assert!(!syncing.get());
            assert!(update_results(
                &model,
                &results,
                &selection,
                &syncing,
                items[1..].to_vec()
            ));
            assert!(selection.selection().is_empty());
            assert!(!update_results(
                &model,
                &results,
                &selection,
                &syncing,
                vec![]
            ));
        },
    );
}
