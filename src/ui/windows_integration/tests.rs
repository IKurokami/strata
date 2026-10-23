// SPDX-License-Identifier: MIT

use super::*;

struct Recording(Rc<RefCell<Vec<bool>>>);

impl Backend for Recording {
    fn apply(&self, enabled: bool) -> io::Result<()> {
        self.0.borrow_mut().push(enabled);
        Ok(())
    }
}

struct Unavailable(Rc<RefCell<Vec<bool>>>);

impl Backend for Unavailable {
    fn apply(&self, enabled: bool) -> io::Result<()> {
        self.0.borrow_mut().push(enabled);
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "fixture failure",
        ))
    }
}

#[test]
fn failed_registration_never_turns_on_an_opted_out_preference() {
    crate::test_support::gtk_test(
        "ui::windows_integration::tests::failed_registration_never_turns_on_an_opted_out_preference",
        || {
            let manager = PreferenceManager::shared();
            manager.set_replace_file_explorer(false);
            let calls = Rc::new(RefCell::new(Vec::new()));
            BACKEND.with(|backend| backend.replace(Rc::new(Unavailable(calls.clone()))));
            APPLIED.with(|state| state.set(None));
            let application = gtk::Application::builder()
                .application_id("io.github.lgse.Strata.ShellFailureTest")
                .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gtk::gio::Cancellable>)
                .expect("application");
            let window = gtk::ApplicationWindow::new(&application);
            bind(&window, &manager);
            assert!(!manager.replace_file_explorer());
            manager.set_replace_file_explorer(true);
            assert!(
                !manager.replace_file_explorer(),
                "failed activation restores the prior choice"
            );
            assert_eq!(*calls.borrow(), vec![false, true]);
            for window in application.windows() {
                window.destroy();
            }
        },
    );
}

#[test]
fn saved_shell_preference_applies_before_settings_and_updates_once_across_windows() {
    crate::test_support::gtk_test(
        "ui::windows_integration::tests::saved_shell_preference_applies_before_settings_and_updates_once_across_windows",
        || {
            crate::ui::preferences::fixtures::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let calls = Rc::new(RefCell::new(Vec::new()));
            BACKEND.with(|backend| backend.replace(Rc::new(Recording(calls.clone()))));
            APPLIED.with(|state| state.set(None));
            let application = gtk::Application::builder()
                .application_id("io.github.lgse.Strata.ShellPreferenceTest")
                .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gtk::gio::Cancellable>)
                .expect("register application");
            let first = gtk::ApplicationWindow::new(&application);
            let second = gtk::ApplicationWindow::new(&application);
            bind(&first, &manager);
            bind(&second, &manager);
            assert_eq!(*calls.borrow(), vec![true]);
            manager.set_replace_file_explorer(false);
            assert_eq!(
                *calls.borrow(),
                vec![true, false],
                "restoration happens during dispatch"
            );
            first.destroy();
            let rebuilt = gtk::ApplicationWindow::new(&application);
            bind(&rebuilt, &manager);
            assert_eq!(*calls.borrow(), vec![true, false]);
            manager.set_replace_file_explorer(true);
            assert_eq!(*calls.borrow(), vec![true, false, true]);
            second.destroy();
            rebuilt.destroy();
        },
    );
}
