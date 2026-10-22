// SPDX-License-Identifier: MIT

use super::*;
use crate::{model::Location, ui::window::present_target};

fn wait_for_navigation(browser: &BrowserView, location: &Location) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while browser.browser().active_location().as_ref() != Some(location)
        || browser
            .browser()
            .active_depth()
            .and_then(|depth| browser.browser().column_snapshot(depth))
            .is_none_or(|column| column.loading)
    {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "navigation did not finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn repeated_activation_reuses_one_window_and_preserves_existing_tabs() {
    crate::test_support::gtk_test(
        "ui::windows_runtime::tests::repeated_activation_reuses_one_window_and_preserves_existing_tabs",
        || {
            let application = gtk::Application::builder()
                .application_id("io.github.lgse.Strata.ReactivationTest")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gio::Cancellable>)
                .expect("application");
            let manager = PreferenceManager::shared();
            manager.set_restore_tabs(true);
            manager.set_keep_running_in_background(false);
            let directory = tempfile::tempdir().expect("fixture directory");
            let location = Location::local(directory.path());
            let browser = present_target(
                &application,
                Some(location.clone()),
                Vec::new(),
                false,
                true,
                false,
            );
            wait_for_navigation(&browser, &location);
            let original = application.windows().pop().expect("browser window");
            let request = serde_json::to_vec(&Request::Open {
                cwd: directory.path().as_os_str().encode_wide().collect(),
                arguments: Vec::new(),
            })
            .expect("activation packet");
            for _ in 0..2 {
                dispatch(&application, &request);
                assert_eq!(application.windows().len(), 1);
                assert!(original.is_visible());
                assert_eq!(browser.browser().active_location(), Some(location.clone()));
            }
            super::super::present(&application);
            assert_eq!(application.windows().len(), 1);
            let selected = browser
                .browser()
                .selected_entries()
                .iter()
                .map(|entry| entry.location.clone())
                .collect::<Vec<_>>();
            let same_folder = serde_json::to_vec(&Request::Open {
                cwd: directory.path().as_os_str().encode_wide().collect(),
                arguments: vec![directory.path().as_os_str().encode_wide().collect()],
            })
            .unwrap();
            for _ in 0..12 {
                dispatch(&application, &same_folder);
                assert!(
                    browser
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading),
                    "opening the loaded folder must not restart loading"
                );
                assert_eq!(
                    browser
                        .browser()
                        .selected_entries()
                        .iter()
                        .map(|entry| entry.location.clone())
                        .collect::<Vec<_>>(),
                    selected
                );
                assert_eq!(application.windows().len(), 1);
            }
            let child = directory.path().join("explicit target");
            std::fs::create_dir(&child).expect("child directory");
            dispatch(
                &application,
                &serde_json::to_vec(&Request::Open {
                    cwd: directory.path().as_os_str().encode_wide().collect(),
                    arguments: vec![child.as_os_str().encode_wide().collect()],
                })
                .expect("target packet"),
            );
            let child_location = Location::local(&child);
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while super::super::tabs_session::load_restorable()
                .is_none_or(|session| !session.tabs.contains(&child_location))
            {
                assert!(
                    std::time::Instant::now() < deadline,
                    "external tab did not load"
                );
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(5));
            }
            let session = super::super::tabs_session::load_restorable().unwrap();
            for _ in 0..12 {
                dispatch(
                    &application,
                    &serde_json::to_vec(&Request::Open {
                        cwd: directory.path().as_os_str().encode_wide().collect(),
                        arguments: vec![child.as_os_str().encode_wide().collect()],
                    })
                    .unwrap(),
                );
                assert_eq!(application.windows().len(), 1);
                assert_eq!(
                    super::super::tabs_session::load_restorable().unwrap().tabs,
                    session.tabs
                );
            }
            assert_eq!(application.windows().len(), 1);
            assert_eq!(browser.browser().active_location(), Some(location));
            for window in application.windows() {
                window.destroy();
            }
        },
    );
}

#[test]
fn secondary_claim_forwards_without_starting_another_application() {
    crate::test_support::gtk_test(
        "ui::windows_runtime::tests::secondary_claim_forwards_without_starting_another_application",
        || {
            let server = Server::bind(&activation_scope()).expect("primary");
            for argument in ["C:\\", "--quit"] {
                assert!(matches!(
                    claim(&[
                        OsString::from("different-installation.exe"),
                        OsString::from(argument)
                    ]),
                    Err(glib::ExitCode::SUCCESS)
                ));
                assert!(server.receive().is_some());
            }
            assert!(matches!(
                claim(&[OsString::from("strata.exe"), OsString::from("--invalid")]),
                Err(glib::ExitCode::FAILURE)
            ));
            assert!(server.receive().is_none());
        },
    );
}

#[test]
fn explicit_windows_target_is_saved_for_the_next_plain_launch() {
    crate::test_support::gtk_test(
        "ui::windows_runtime::tests::explicit_windows_target_is_saved_for_the_next_plain_launch",
        || {
            let application = gtk::Application::builder()
                .application_id("io.github.lgse.Strata.SessionTest")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gio::Cancellable>)
                .expect("application");
            let manager = PreferenceManager::shared();
            manager.set_restore_tabs(true);
            manager.set_keep_running_in_background(false);
            let directory = tempfile::tempdir().expect("session fixture");
            let last = directory.path().join("last folder 中文");
            std::fs::create_dir(&last).expect("last folder");
            let browser = present_target(
                &application,
                Some(Location::local(directory.path())),
                Vec::new(),
                false,
                true,
                false,
            );
            wait_for_navigation(&browser, &Location::local(directory.path()));
            browser.navigate_location(Location::local(&last));
            wait_for_navigation(&browser, &Location::local(&last));
            assert_eq!(
                super::super::tabs_session::load_restorable()
                    .expect("saved session")
                    .tabs,
                vec![Location::local(&last)]
            );
            browser
                .widget()
                .root()
                .and_downcast::<gtk::Window>()
                .expect("window")
                .close();
            let restored = present_target(&application, None, Vec::new(), false, true, true);
            wait_for_navigation(&restored, &Location::local(&last));
            for window in application.windows() {
                window.destroy();
            }
        },
    );
}

#[test]
fn user_navigation_wins_over_a_pending_startup_session() {
    crate::test_support::gtk_test(
        "ui::windows_runtime::tests::user_navigation_wins_over_a_pending_startup_session",
        || {
            let application = gtk::Application::builder()
                .application_id("io.github.lgse.Strata.SessionRaceTest")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gio::Cancellable>)
                .expect("application");
            let manager = PreferenceManager::shared();
            manager.set_restore_tabs(true);
            let root = tempfile::tempdir().expect("session fixture");
            let saved = root.path().join("saved");
            let chosen = root.path().join("chosen");
            for path in [&saved, &chosen] {
                std::fs::create_dir(path).expect("folder");
            }
            super::super::tabs_session::save(&[Location::local(&saved)], 0);
            let browser = present_target(&application, None, Vec::new(), false, true, true);
            browser.navigate_location(Location::local(&chosen));
            wait_for_navigation(&browser, &Location::local(&chosen));
            let deadline = std::time::Instant::now() + Duration::from_millis(300);
            while std::time::Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                browser.browser().active_location(),
                Some(Location::local(&chosen))
            );
            for window in application.windows() {
                window.destroy();
            }
        },
    );
}

#[test]
fn retained_window_reopens_and_live_preference_releases_only_hidden_windows() {
    crate::test_support::gtk_test(
        "ui::windows_runtime::tests::retained_window_reopens_and_live_preference_releases_only_hidden_windows",
        || {
            let application = gtk::Application::builder()
                .application_id("io.github.lgse.Strata.ResidentTest")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            install(
                &application,
                Server::bind(&activation_scope()).expect("test activation endpoint"),
            );
            application
                .register(None::<&gio::Cancellable>)
                .expect("register application");
            crate::ui::preferences::fixtures::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            assert!(
                manager.keep_running_in_background(),
                "saved behavior applies before Settings opens"
            );
            manager.set_restore_tabs(false);
            let directory = tempfile::tempdir().expect("fixture directory");
            let first_browser = present_target(
                &application,
                Some(Location::local(directory.path())),
                Vec::new(),
                false,
                true,
                false,
            );
            let first = first_browser
                .widget()
                .root()
                .and_downcast::<gtk::Window>()
                .expect("first window");
            first.close();
            assert!(!first.is_visible());
            assert!(application.windows().contains(&first));
            let (scope, request) = shell_activation()
                .expect("shell activation")
                .expect("primary endpoint available");
            activation::forward(&scope, &request).expect("direct helper activation");
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !first.is_visible() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "direct activation timed out"
                );
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                application.windows().len(),
                1,
                "reuse creates no extra window"
            );
            assert!(first.is_visible());
            first.close();
            manager.set_keep_running_in_background(false);
            assert!(!application.windows().contains(&first));
            let second_browser = present_target(
                &application,
                Some(Location::local(directory.path())),
                Vec::new(),
                false,
                true,
                false,
            );
            let second = application.windows().pop().expect("replacement window");
            manager.set_keep_running_in_background(true);
            manager.set_keep_running_in_background(false);
            assert!(second.is_visible());
            assert!(application.windows().contains(&second));
            second_browser.set_view_mode(crate::ui::browser_modes::BrowserMode::Icons);
            manager.set_keep_running_in_background(true);
            let operations = Rc::new(crate::test_support::operations::HeldOperations::default());
            second_browser
                .browser()
                .set_operation_provider(operations.clone());
            second_browser.browser().transfer(
                Location::local(directory.path()),
                vec![crate::services::PasteItem {
                    source: Location::local(directory.path().join("held-copy.txt")),
                    conflict: crate::services::TransferConflict::FailIfExists,
                }],
                false,
                false,
            );
            let id = second_browser
                .browser()
                .last_started_operation()
                .expect("held transfer");
            second.close();
            assert!(
                second.is_visible(),
                "closing must not hide an active transfer"
            );
            quit(&application);
            assert!(
                application.windows().contains(&second),
                "explicit exit retains operation guards"
            );
            operations.emit(
                id,
                crate::services::OperationEvent::Pasted {
                    request_id: id,
                    locations: Vec::new(),
                },
            );
            manager.set_keep_running_in_background(false);
            second.close();
            assert!(application.windows().is_empty());
        },
    );
}

#[test]
fn activation_resolves_relative_paths_in_the_senders_directory() {
    crate::test_support::gtk_test(
        "ui::windows_runtime::tests::activation_resolves_relative_paths_in_the_senders_directory",
        || {
            let application = gtk::Application::builder()
                .application_id("io.github.lgse.Strata.ActivationTest")
                .flags(gio::ApplicationFlags::NON_UNIQUE)
                .build();
            install(
                &application,
                Server::bind(&format!("activation-ui-test-{}", std::process::id()))
                    .expect("test activation endpoint"),
            );
            application
                .register(None::<&gio::Cancellable>)
                .expect("register application");
            let directory = tempfile::tempdir().expect("fixture directory");
            let child = directory.path().join("thư mục # 1");
            std::fs::create_dir(&child).expect("fixture child");
            let browser = present_target(
                &application,
                Some(Location::local(directory.path())),
                Vec::new(),
                false,
                true,
                false,
            );
            PreferenceManager::shared().set_keep_running_in_background(true);
            browser
                .widget()
                .root()
                .and_downcast::<gtk::Window>()
                .expect("window")
                .close();
            dispatch(&application, b"not json");
            dispatch(
                &application,
                &serde_json::to_vec(&Request::Open {
                    cwd: Vec::new(),
                    arguments: Vec::new(),
                })
                .expect("invalid packet"),
            );
            assert!(
                application
                    .windows()
                    .iter()
                    .all(|window| !window.is_visible())
            );
            let request = Request::Open {
                cwd: directory.path().as_os_str().encode_wide().collect(),
                arguments: vec![OsString::from("thư mục # 1").encode_wide().collect()],
            };
            dispatch(
                &application,
                &serde_json::to_vec(&request).expect("activation packet"),
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while super::super::tabs_session::load_restorable()
                .is_none_or(|session| !session.tabs.contains(&Location::local(&child)))
            {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "relative activation did not navigate"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                application
                    .windows()
                    .iter()
                    .any(|window| window.is_visible())
            );
            let entry = gtk::Entry::new();
            let input_window = gtk::ApplicationWindow::builder()
                .application(&application)
                .child(&entry)
                .build();
            input_window.present();
            assert!(entry.grab_focus(), "input can own the Win32 IME context");
            dispatch(
                &application,
                &serde_json::to_vec(&Request::Quit).expect("quit packet"),
            );
            assert!(application.windows().is_empty());
        },
    );
}
