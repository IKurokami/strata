// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{
    browser::{BrowserView, PeekBehavior},
    browser_modes::BrowserMode,
    preferences::{PreferenceManager, fixtures::seed_saved_preferences_for_test},
    windows_open::capture_properties_for_test,
};

struct Capture;
impl Drop for Capture {
    fn drop(&mut self) {
        capture_properties_for_test(None);
    }
}

fn shows_title(widget: &gtk::Widget, title: &str) -> bool {
    if widget
        .downcast_ref::<gtk::Label>()
        .is_some_and(|label| label.text() == title)
    {
        return true;
    }
    let mut next = widget.first_child();
    while let Some(child) = next {
        if shows_title(&child, title) {
            return true;
        }
        next = child.next_sibling();
    }
    false
}

#[test]
fn windows_properties_dispatch_uses_saved_and_live_choice_across_views() {
    crate::test_support::gtk_test(
        "ui::browser::properties::tests::windows_properties_dispatch_uses_saved_and_live_choice_across_views",
        || {
            seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            assert!(manager.windows_properties());
            let _capture = Capture;
            capture_properties_for_test(Some(Vec::new()));
            let folder = tempfile::tempdir().expect("properties fixture");
            let location = Location::local(folder.path().to_path_buf());
            let views = [
                BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                ),
                BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                ),
            ];
            let windows = views.each_ref().map(|view| {
                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&view.widget()));
                gtk::Window::builder().child(&overlay).build()
            });
            for view in &views {
                view.state.show_folder_properties(&location);
            }
            assert_eq!(
                capture_properties_for_test(Some(Vec::new())).expect("capture"),
                vec![vec![folder.path().to_path_buf()]; 2]
            );
            manager.set_windows_properties(false);
            for (view, window) in views.iter().zip(&windows) {
                view.state.show_folder_properties(&location);
                assert!(shows_title(window.upcast_ref(), &location.display_name()));
            }
            assert!(
                capture_properties_for_test(Some(Vec::new()))
                    .expect("capture")
                    .is_empty()
            );
            views[1].set_view_mode(BrowserMode::Icons);
            manager.set_windows_properties(true);
            for view in &views {
                view.state.show_folder_properties(&location);
            }
            assert_eq!(
                capture_properties_for_test(Some(Vec::new())).expect("capture"),
                vec![vec![folder.path().to_path_buf()]; 2]
            );
            for window in windows {
                window.close();
            }
        },
    );
}
