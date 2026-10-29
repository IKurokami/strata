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
                    view.state.request_windows_delete(vec![selected.clone()]);
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
