// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::gtk_test;

#[test]
fn update_restart_exits_retained_windows_only_after_jobs_allow_it() {
    gtk_test(
        "ui::settings::restart_windows_tests::update_restart_exits_retained_windows_only_after_jobs_allow_it",
        || {
            let application = gtk::Application::builder()
                .application_id("org.strata.WindowsUpdateRestartTest")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gio::Cancellable>)
                .expect("register restart application");
            let button = gtk::Button::new();
            restart_application(&button);
            let first = gtk::ApplicationWindow::builder()
                .application(&application)
                .child(&button)
                .build();
            let second = gtk::ApplicationWindow::builder()
                .application(&application)
                .build();
            // Closing a retained browser window does not exit its application.
            first.connect_close_request(|_| glib::Propagation::Stop);
            second.connect_close_request(|_| glib::Propagation::Stop);
            let busy = Rc::new(Cell::new(true));
            let jobs = busy.clone();
            crate::ui::close_guard::install(&second, move |exiting| {
                (exiting && jobs.get()).then(|| crate::ui::close_guard::CloseBlocker {
                    title: "Busy".to_owned(),
                    detail: String::new(),
                })
            });
            first.present();
            second.present();

            restart_application(&button);
            assert_eq!(application.windows().len(), 2, "running jobs block restart");
            busy.set(false);
            restart_application(&button);
            assert!(
                application.windows().is_empty(),
                "exit releases the bundle for the handoff"
            );
        },
    );
}
