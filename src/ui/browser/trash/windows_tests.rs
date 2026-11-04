// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    services::{OperationEvent, PermanentDeleteMethod},
    test_support::operations::{HeldOperations, entry},
    ui::{
        browser::{BrowserView, PeekBehavior},
        browser_modes::BrowserMode,
        preferences::{PreferenceManager, fixtures::seed_saved_preferences_for_test},
    },
};

#[test]
fn windows_delete_recycles_across_saved_method_changes_and_view_rebuilds() {
    crate::test_support::gtk_test(
        "ui::browser::trash::windows_tests::windows_delete_recycles_across_saved_method_changes_and_view_rebuilds",
        || {
            seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            assert_eq!(
                manager.permanent_delete_method(),
                PermanentDeleteMethod::Native
            );
            let views = std::array::from_fn::<_, 2, _>(|_| {
                BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                )
            });
            let fixture = tempfile::tempdir().expect("delete fixture");
            let path = fixture.path().join("keep.txt");
            std::fs::write(&path, b"held operation").expect("fixture file");
            let selected = entry(Location::local(path.clone()));
            for (index, method) in [
                PermanentDeleteMethod::Native,
                PermanentDeleteMethod::Fast,
                PermanentDeleteMethod::Native,
            ]
            .into_iter()
            .enumerate()
            {
                if index > 0 {
                    manager.set_permanent_delete_method(method);
                }
                for view in &views {
                    if index == 2 {
                        view.set_view_mode(BrowserMode::Icons);
                    }
                    let operations = Rc::new(HeldOperations::default());
                    view.set_operation_provider(operations.clone());
                    if index == 1 {
                        view.state.request_delete(vec![selected.clone()], false);
                    } else {
                        view.state.request_windows_delete(vec![selected.clone()]);
                    }
                    let requests = operations.delete_requests.borrow();
                    assert_eq!(requests.len(), 1);
                    let (id, permanent, count) = requests[0];
                    assert!(!permanent);
                    assert_eq!(count, 1);
                    assert_eq!(
                        operations.delete_methods.borrow().as_slice(),
                        &[PermanentDeleteMethod::Native]
                    );
                    assert_eq!(manager.permanent_delete_method(), method);
                    drop(requests);
                    operations.emit(
                        id,
                        OperationEvent::Deleted {
                            request_id: id,
                            locations: vec![selected.location.clone()],
                        },
                    );
                }
            }
            assert!(path.exists(), "held requests never delete the fixture");
        },
    );
}

#[test]
fn default_delete_recycles_and_permanent_delete_waits_for_confirmation() {
    crate::test_support::gtk_test(
        "ui::browser::trash::windows_tests::default_delete_recycles_and_permanent_delete_waits_for_confirmation",
        || {
            fn wait(condition: impl Fn() -> bool) {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while !condition() {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "delete action did not settle"
                    );
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            fn has_confirmation(widget: &gtk::Widget) -> bool {
                if widget.has_css_class("app-modal-layer") && widget.is_visible() {
                    return true;
                }
                let mut child = widget.first_child();
                while let Some(widget) = child {
                    child = widget.next_sibling();
                    if has_confirmation(&widget) {
                        return true;
                    }
                }
                false
            }
            for permanent in [false, true] {
                seed_saved_preferences_for_test();
                PreferenceManager::shared()
                    .set_permanent_delete_method(PermanentDeleteMethod::Fast);
                let fixture = tempfile::tempdir().unwrap();
                let path = fixture.path().join("keep.txt");
                std::fs::write(&path, b"held operation").unwrap();
                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                view.set_view_mode(BrowserMode::Columns);
                let operations = Rc::new(HeldOperations::default());
                view.set_operation_provider(operations.clone());
                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&view.widget()));
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                let browser = view.browser();
                browser.navigate(Location::local(fixture.path()));
                wait(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading && column.count == 1)
                });
                browser.select(0, 0);
                browser.focus_active();
                assert!(view.confirm_delete(permanent));
                if permanent {
                    wait(|| has_confirmation(overlay.upcast_ref()));
                    assert!(operations.delete_requests.borrow().is_empty());
                } else {
                    wait(|| !operations.delete_requests.borrow().is_empty());
                    let requests = operations.delete_requests.borrow();
                    assert_eq!(requests.len(), 1);
                    assert!(!requests[0].1);
                    assert_eq!(requests[0].2, 1);
                    assert_eq!(
                        operations.delete_methods.borrow().as_slice(),
                        &[PermanentDeleteMethod::Native]
                    );
                }
                assert!(path.exists());
                browser.clear_observer();
                window.destroy();
            }
        },
    );
}
