// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn result_reconciliation_preserves_identity_selection_and_anchor_across_reordering() {
    crate::test_support::gtk_test(
        "ui::inline_search::collection::tests::result_reconciliation_preserves_identity_selection_and_anchor_across_reordering",
        || {
            let (collection, _scroller, _overlay) = build_collection(
                SearchPresentation::Rows,
                Rc::new(Cell::new(true)),
                PathBuf::from("/search"),
                CollectionBehavior {
                    multiple_selection: Rc::new(Cell::new(true)),
                    activate: Rc::new(|_| {}),
                    single_click: Rc::new(|_| {}),
                    focus_items: Rc::new(|| {}),
                },
            );
            let mut items: Vec<_> = ["a.txt", "b.txt", "c.txt"]
                .into_iter()
                .map(|name| SearchItem::for_test(PathBuf::from("/search").join(name), false))
                .collect();
            collection.update(&items, true);
            collection.selection.select_item(1, true);
            collection.selection.select_item(2, false);
            collection.anchor.set(Some(1));
            let c = collection.sorted.item(2).expect("stable result");
            items.swap(0, 2);
            items[1].name = "renamed result".into();
            items.push(SearchItem::for_test(PathBuf::from("/search/d.txt"), false));
            collection.update(&items, true);
            assert_eq!(collection.sorted.item(0).as_ref(), Some(&c));
            assert_eq!(
                collection.item(1).expect("updated result").name,
                "renamed result"
            );
            assert_eq!(collection.selected_positions(), [0, 1]);
            assert_eq!(collection.anchor.get(), Some(1));
            assert_eq!(
                collection
                    .items()
                    .iter()
                    .map(|item| &item.path)
                    .collect::<Vec<_>>(),
                items.iter().map(|item| &item.path).collect::<Vec<_>>()
            );
            collection.update(&items[1..], true);
            assert_eq!(collection.selected_positions(), [0]);
            assert_eq!(collection.anchor.get(), Some(0));
            collection.update(&[], true);
            assert!(collection.selected_positions().is_empty());
            assert_eq!(collection.anchor.get(), None);
        },
    );
}
