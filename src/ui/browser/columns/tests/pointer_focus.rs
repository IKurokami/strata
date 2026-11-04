// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{LoadHandle, PreviewEvent, PreviewProvider, PreviewRequest};
use crate::ui::preview::PreviewDrawer;

#[derive(Default)]
pub(super) struct PreviewCalls(pub(super) Cell<usize>);

impl PreviewProvider for PreviewCalls {
    fn load(&self, _: PreviewRequest, _: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.0.set(self.0.get() + 1);
        LoadHandle::new(|| {})
    }
}

fn named(fixture: &Columns, depth: usize, name: &str) -> (usize, crate::model::FileEntry) {
    (0..)
        .map_while(|pos| {
            fixture
                .view
                .browser()
                .entry_at(depth, pos)
                .map(|entry| (pos, entry))
        })
        .find(|(_, entry)| entry.display_name == name)
        .expect("named fixture entry")
}

fn visible_popover(widget: &gtk::Widget) -> Option<gtk::Popover> {
    if let Ok(popover) = widget.clone().downcast::<gtk::Popover>()
        && popover.is_visible()
    {
        return Some(popover);
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(popover) = visible_popover(&widget) {
            return Some(popover);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn background_deselection_keeps_preview_until_a_real_item_focus_changes() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::pointer_focus::background_deselection_keeps_preview_until_a_real_item_focus_changes",
        || {
            let fixture = Columns::new(2, 1200);
            let provider = Rc::new(PreviewCalls::default());
            let preview = PreviewDrawer::new(provider.clone(), false);
            preview.observe_browser(&fixture.view.browser());
            let (file_pos, file) = named(&fixture, 1, "note.txt");
            fixture.view.browser().select(1, file_pos);
            preview.show(file, Some(1));
            pump_for(Duration::from_millis(80));
            let calls = provider.0.get();
            fixture.view.state.focus_column_background(1);
            fixture.view.state.clear_column_background_selection(1);
            pump_for(Duration::from_millis(80));
            assert!(
                preview.is_open(),
                "background deselection must not dismiss preview"
            );
            fixture.view.state.focus_column_background(1);
            pump_for(Duration::from_millis(80));
            assert!(preview.is_open());
            assert_eq!(
                provider.0.get(),
                calls,
                "background focus must not reload preview"
            );
            assert!(fixture.view.browser().selected_positions(1).is_empty());
            let column = fixture.view.state.columns.borrow()[1].clone();
            assert_eq!(
                gtk::prelude::RootExt::focus(&fixture.window),
                Some(column.listing_scroll.clone().upcast::<gtk::Widget>())
            );
            let (folder_pos, _) = named(&fixture, 1, "level-1");
            fixture.view.browser().select(1, folder_pos);
            assert!(
                !preview.is_open(),
                "real folder focus still dismisses preview"
            );
            fixture.view.browser().select(1, file_pos);
            pump_until(|| preview.is_open(), "primary file preview");
            assert!(provider.0.get() > calls);
        },
    );
}

#[test]
fn background_context_menu_returns_focus_to_the_viewport_not_the_old_row() {
    crate::test_support::gtk_test(
        "ui::browser::columns::tests::pointer_focus::background_context_menu_returns_focus_to_the_viewport_not_the_old_row",
        || {
            let fixture = Columns::new(2, 1200);
            let directory = fixture._root.path().join("level-0");
            for index in 0..100 {
                std::fs::write(directory.join(format!("file-{index:03}.txt")), "text")
                    .expect("file fixture");
            }
            fixture.view.browser().reload_active();
            pump_until(
                || {
                    fixture.view.browser().entry_at(1, 100).is_some()
                        && column_loaded(&fixture.view, 1)
                },
                "long listing",
            );
            let (position, file) = named(&fixture, 1, "note.txt");
            let provider = Rc::new(PreviewCalls::default());
            let preview = PreviewDrawer::new(provider.clone(), false);
            preview.observe_browser(&fixture.view.browser());
            fixture.view.browser().select(1, position);
            preview.show(file.clone(), Some(1));
            pump_for(Duration::from_millis(100));
            let calls = provider.0.get();
            let column = fixture.view.state.columns.borrow()[1].clone();
            let adjustment = column.listing_scroll.vadjustment();
            adjustment.set_value(adjustment.lower());
            restore_column_cursor(
                &column,
                column.map.view_position(position).expect("file row"),
            );
            (column.folder_context_trigger)(20.0, 20.0);
            pump_until(
                || visible_popover(&fixture.view.widget()).is_some(),
                "background menu",
            );
            let saved = fixture
                .view
                .state
                .context_menu_focus
                .borrow()
                .as_ref()
                .and_then(glib::WeakRef::upgrade)
                .expect("pre-menu focus");
            assert_eq!(saved, column.listing_scroll.clone().upcast::<gtk::Widget>());
            assert_eq!(
                fixture
                    .view
                    .browser()
                    .focused_entry()
                    .expect("focused file")
                    .location,
                file.location
            );
            let menu = visible_popover(&fixture.view.widget()).expect("menu");
            menu.popdown();
            pump_for(Duration::from_millis(200));
            assert_eq!(
                gtk::prelude::RootExt::focus(&fixture.window),
                Some(column.listing_scroll.clone().upcast::<gtk::Widget>()),
                "dismissal must not refocus an offscreen row"
            );
            assert!(preview.is_open());
            assert_eq!(provider.0.get(), calls);
            assert_eq!(
                fixture
                    .view
                    .browser()
                    .focused_entry()
                    .expect("logical cursor")
                    .location,
                file.location
            );
            assert!(fixture.view.item_view_has_focus());
            fixture.view.keyboard_navigation();
            assert!(fixture.view.page_selection(-1, false));
            assert_ne!(
                fixture
                    .view
                    .browser()
                    .focused_entry()
                    .expect("page navigation")
                    .location,
                file.location
            );
            fixture.view.state.focus_column_background(1);
            assert!(fixture.view.jump_selection(-1));
            assert_eq!(
                fixture
                    .view
                    .browser()
                    .focused_item()
                    .expect("first entry")
                    .1,
                0
            );
        },
    );
}
