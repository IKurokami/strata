// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn login_startup_keeps_only_a_tray_lifetime_until_a_real_window_opens() {
    crate::test_support::gtk_test(
        "ui::windows_desktop::tests::login_startup_keeps_only_a_tray_lifetime_until_a_real_window_opens",
        || {
            let application = gtk::Application::builder()
                .application_id("io.github.lgse.Strata.TrayStartupTest")
                .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gtk::gio::Cancellable>)
                .expect("private app");
            let preferences = PreferenceManager::shared();
            let retained = Rc::downgrade(&preferences);
            start_background(&application);
            drop(preferences);
            let until = std::time::Instant::now() + std::time::Duration::from_millis(300);
            while std::time::Instant::now() < until {
                gtk::glib::MainContext::default().iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert!(
                retained.upgrade().is_some(),
                "tray lost its saved-preference bindings"
            );
            start_background(&application);
            assert!(
                application.windows().is_empty(),
                "login startup created an application window"
            );
            BACKGROUND.with(|slot| {
                assert!(
                    !slot
                        .borrow()
                        .as_ref()
                        .expect("tray lifetime")
                        .0
                        .is_realized()
                )
            });
            let window = gtk::ApplicationWindow::new(&application);
            BACKGROUND.with(|slot| assert!(slot.borrow().is_none()));
            window.present();
            start_background(&application);
            assert!(
                window.is_visible(),
                "a later startup request hid the user's window"
            );
            window.destroy();
        },
    );
}

#[test]
fn saved_console_binding_applies_before_settings_and_survives_window_rebuilds() {
    crate::test_support::gtk_test(
        "ui::windows_desktop::tests::saved_console_binding_applies_before_settings_and_survives_window_rebuilds",
        || {
            crate::ui::preferences::fixtures::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            APPLIED.with(|state| state.set(None));
            CALLS.with(|calls| calls.borrow_mut().clear());
            let application = gtk::Application::builder()
                .application_id("io.github.lgse.Strata.ConsoleTest")
                .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gtk::gio::Cancellable>)
                .expect("application");
            let first = gtk::ApplicationWindow::new(&application);
            let second = gtk::ApplicationWindow::new(&application);
            bind(&first, &manager);
            bind(&second, &manager);
            CALLS.with(|calls| assert_eq!(*calls.borrow(), vec![true]));
            manager.set_show_console(false);
            CALLS.with(|calls| assert_eq!(*calls.borrow(), vec![true, false]));
            first.destroy();
            let rebuilt = gtk::ApplicationWindow::new(&application);
            bind(&rebuilt, &manager);
            CALLS.with(|calls| assert_eq!(*calls.borrow(), vec![true, false]));
            manager.set_show_console(true);
            CALLS.with(|calls| assert_eq!(*calls.borrow(), vec![true, false, true]));
            second.destroy();
            rebuilt.destroy();
        },
    );
}
