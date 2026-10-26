// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{LoadHandle, PreviewEvent, PreviewProvider, PreviewRequest};
use crate::ui::{
    browser::{BrowserView, PeekBehavior},
    browser_modes::BrowserMode,
    preview::PreviewDrawer,
};

#[derive(Default)]
struct PreviewCalls(RefCell<Vec<String>>);

impl PreviewProvider for PreviewCalls {
    fn load(&self, request: PreviewRequest, _: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.0.borrow_mut().push(request.entry.display_name);
        LoadHandle::new(|| {})
    }
}

fn settle_callbacks() {
    let until = Instant::now() + Duration::from_millis(200);
    while Instant::now() < until {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn item_context_focus_and_dismissal_preserve_preview_until_primary_selection_changes() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::preview::item_context_focus_and_dismissal_preserve_preview_until_primary_selection_changes",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for grouped in [false, true] {
                    let view = BrowserView::new(
                        Rc::new(super::menus::MenuSource),
                        PeekBehavior::default(),
                    );
                    view.set_view_mode(mode);
                    let provider = Rc::new(PreviewCalls::default());
                    let preview = PreviewDrawer::new(provider.clone(), false);
                    preview.observe_browser(&view.browser());
                    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    content.append(&view.widget());
                    content.append(&preview.widget());
                    let overlay = gtk::Overlay::new();
                    overlay.set_child(Some(&content));
                    let window = gtk::Window::builder()
                        .default_width(1100)
                        .default_height(700)
                        .child(&overlay)
                        .build();
                    window.present();
                    view.browser().navigate(Location::local("/fixture"));
                    wait_until(|| view.browser().entry_at(0, 7).is_some());
                    let position = |name: &str| {
                        (0..)
                            .map_while(|index| {
                                view.browser()
                                    .entry_at(0, index)
                                    .map(|entry| (index, entry))
                            })
                            .find(|(_, entry)| entry.display_name == name)
                            .expect("fixture entry")
                    };
                    let (first_position, first) = position("notes.txt");
                    let (other_position, _) = position("other.txt");
                    let positions = if grouped {
                        vec![first_position, other_position]
                    } else {
                        vec![first_position]
                    };
                    view.browser()
                        .set_selection(0, &positions, Some(first_position));
                    preview.show(first, Some(0));
                    settle_callbacks();
                    provider.0.borrow_mut().clear();
                    let title = super::menus::descendants(&preview.widget())
                        .into_iter()
                        .find_map(|widget| {
                            widget
                                .downcast::<gtk::Label>()
                                .ok()
                                .filter(|label| label.has_css_class("preview-title"))
                        })
                        .expect("preview title");
                    let menu = super::menus::open_menu(&view, Some("other.txt"));
                    settle_callbacks();
                    assert!(
                        provider.0.borrow().is_empty(),
                        "{mode:?}, grouped={grouped}: context focus changed preview: {:?}",
                        provider.0.borrow()
                    );
                    assert_eq!(title.text(), "notes.txt");
                    menu.popdown();
                    settle_callbacks();
                    assert!(
                        provider.0.borrow().is_empty(),
                        "{mode:?}, grouped={grouped}: popup focus restoration changed preview: {:?}",
                        provider.0.borrow()
                    );
                    assert_eq!(title.text(), "notes.txt");
                    view.browser().select(0, other_position);
                    wait_until(|| !provider.0.borrow().is_empty());
                    assert_eq!(&*provider.0.borrow(), &["other.txt"]);
                    view.browser().clear_observer();
                    window.destroy();
                    settle_callbacks();
                }
            }
        },
    );
}
