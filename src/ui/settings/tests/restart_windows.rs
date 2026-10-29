// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::gtk_test;

#[test]
fn preference_restart_honors_guards_and_exits_retained_windows() {
    gtk_test(
        "ui::settings::restart_windows_tests::preference_restart_honors_guards_and_exits_retained_windows",
        || {
            let application = gtk::Application::builder()
                .application_id("org.strata.WindowsPreferenceRestartTest")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gio::Cancellable>)
                .expect("register");
            let window = gtk::ApplicationWindow::builder()
                .application(&application)
                .build();
            window.connect_close_request(|_| glib::Propagation::Stop);
            let busy = Rc::new(Cell::new(true));
            let guard = busy.clone();
            crate::ui::close_guard::install(&window, move |exiting| {
                (exiting && guard.get()).then(|| crate::ui::close_guard::CloseBlocker {
                    title: "Busy".into(),
                    detail: String::new(),
                })
            });
            window.present();
            restart_with_waiter(Some(&application), |_, _| {
                panic!("blocked restart must not prepare a helper")
            });
            assert!(application.windows().contains(&window.clone().upcast()));
            busy.set(false);
            restart_with_waiter(Some(&application), |_, _| {
                let executable =
                    std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("Windows root"))
                        .join("System32/cmd.exe");
                let mut command = Command::new(executable);
                command.args(["/D", "/C", "exit", "0"]);
                crate::background_process::configure(&mut command);
                Some(command)
            });
            assert!(
                application.windows().is_empty(),
                "explicit restart bypasses background retention"
            );
        },
    );
}

#[test]
fn restart_failure_keeps_windows_open_and_none_application_does_not_spawn() {
    gtk_test(
        "ui::settings::restart_windows_tests::restart_failure_keeps_windows_open_and_none_application_does_not_spawn",
        || {
            restart_with_waiter(None, |_, _| panic!("closed application must not spawn"));
            let application = gtk::Application::builder()
                .application_id("org.strata.WindowsRestartFailureTest")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gio::Cancellable>)
                .expect("register");
            let window = gtk::ApplicationWindow::builder()
                .application(&application)
                .child(&gtk::Overlay::new())
                .build();
            window.present();
            restart_with_waiter(Some(&application), |_, _| {
                Some(Command::new("C:/strata-missing-restart-fixture.exe"))
            });
            assert!(window.is_visible());
            assert!(application.windows().contains(&window.clone().upcast()));
            window.destroy();
        },
    );
}

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
