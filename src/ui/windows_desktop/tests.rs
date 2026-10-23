// SPDX-License-Identifier: MIT

use super::*;

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
