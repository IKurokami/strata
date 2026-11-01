// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::{browser::BrowserView, browser_modes::BrowserMode, motion};
use std::time::{Duration, Instant};

struct CancellableProvider {
    requests: Cell<usize>,
    cancelled: Rc<Cell<usize>>,
}

impl PreviewProvider for CancellableProvider {
    fn load(&self, _request: PreviewRequest, _emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.requests.set(self.requests.get() + 1);
        let cancelled = self.cancelled.clone();
        LoadHandle::new(move || cancelled.set(cancelled.get() + 1))
    }
}

fn entry(name: &str) -> FileEntry {
    crate::test_support::operations::entry(crate::model::Location::local(name))
}

fn settle() {
    let deadline = Instant::now() + motion::PANEL + Duration::from_millis(60);
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(3));
    }
}

#[test]
fn closing_releases_requests_immediately_and_cannot_hide_a_reopened_preview() {
    crate::test_support::gtk_test(
        "ui::preview::closing_tests::closing_releases_requests_immediately_and_cannot_hide_a_reopened_preview",
        || {
            let settings = gtk::Settings::default().expect("GTK settings");
            let enabled = settings.is_gtk_enable_animations();
            settings.set_gtk_enable_animations(true);
            for mode in [BrowserMode::Columns, BrowserMode::List] {
                crate::ui::preferences::PreferenceManager::shared().set_browser_mode(mode);
                for reduced in [false, true] {
                    motion::set_reduce_motion(reduced);
                    let provider = Rc::new(CancellableProvider {
                        requests: Cell::new(0),
                        cancelled: Rc::new(Cell::new(0)),
                    });
                    let preview = PreviewDrawer::new(provider.clone(), false);
                    let browser = BrowserView::new(
                        Rc::new(crate::adapters::LocalFileSource),
                        crate::ui::browser::PeekBehavior::default(),
                    );
                    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
                    let content = gtk::Paned::new(gtk::Orientation::Horizontal);
                    content.set_end_child(Some(&browser.widget()));
                    split.set_start_child(Some(&content));
                    split.set_end_child(Some(&preview.widget()));
                    let window = gtk::Window::builder()
                        .default_width(1200)
                        .default_height(700)
                        .child(&split)
                        .build();
                    preview.attach_split(&split, &content, &browser, None);
                    window.present();
                    for route in 0..3 {
                        preview.show(entry("before.txt"), None);
                        settle();
                        match route {
                            0 => preview
                                .handle_browser_event(&browser.browser(), &BrowserEvent::Reset),
                            1 => {
                                let mut folder = entry("folder");
                                folder.kind = EntryKind::Directory;
                                preview.show_after_focus_change(folder, None);
                            }
                            _ => preview.close(),
                        }
                        assert!(!preview.is_open());
                        assert!(preview.state.current.borrow().is_none());
                        assert!(preview.state.load.borrow().is_none());
                        assert_eq!(
                            provider.cancelled.get(),
                            provider.requests.get(),
                            "closing must release the active request before animation completes"
                        );
                        assert_eq!(preview.state.animating.get(), !reduced);
                        preview.clear_target();
                        preview.show(entry("after.json"), None);
                        settle();
                        assert!(preview.is_open(), "an old close must not hide a new target");
                        assert_eq!(preview.state.title.text(), "after.json");
                        assert!(!preview.state.animating.get());
                    }
                    preview.clear_target();
                    settle();
                    assert!(!preview.state.slot.is_visible());
                    assert!(!preview.state.animating.get());
                    preview.show(entry("unmapping.txt"), None);
                    settle();
                    preview.clear_target();
                    window.destroy();
                    settle();
                    assert!(!preview.state.animating.get());
                    assert!(preview.state.current.borrow().is_none());
                }
            }
            motion::set_reduce_motion(false);
            settings.set_gtk_enable_animations(enabled);
        },
    );
}
