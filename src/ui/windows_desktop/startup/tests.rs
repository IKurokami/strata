// SPDX-License-Identifier: MIT

use super::*;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<bool>>,
    fail_enable: bool,
    hold: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}
impl Backend for Fake {
    fn apply(&self, enabled: bool) -> io::Result<()> {
        self.calls.lock().expect("calls").push(enabled);
        if enabled && let Some((entered, release)) = self.hold.lock().expect("worker gate").take() {
            entered.send(()).expect("worker entered");
            release
                .recv_timeout(Duration::from_secs(5))
                .expect("release worker");
        }
        if self.fail_enable && enabled {
            Err(io::Error::from_raw_os_error(5))
        } else {
            Ok(())
        }
    }
}

fn pump_until(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(
            Instant::now() < deadline,
            "startup operation did not settle"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn application() -> gtk::Application {
    let application = gtk::Application::builder()
        .application_id("io.github.lgse.Strata.StartupTest")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .expect("private application");
    application
}

#[test]
fn saved_startup_applies_once_before_settings_and_changes_across_window_rebuilds() {
    crate::test_support::gtk_test(
        "ui::windows_desktop::startup::tests::saved_startup_applies_once_before_settings_and_changes_across_window_rebuilds",
        || {
            crate::ui::preferences::fixtures::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let backend = Arc::new(Fake::default());
            BACKEND.with(|slot| slot.replace(backend.clone()));
            let application = application();
            let first = gtk::ApplicationWindow::new(&application);
            let second = gtk::ApplicationWindow::new(&application);
            bind(&first, &manager);
            bind(&second, &manager);
            pump_until(|| STATE.with(|state| state.confirmed.get() == Some(false)));
            assert_eq!(*backend.calls.lock().expect("calls"), vec![false]);
            toggle_from_tray(&manager);
            pump_until(|| STATE.with(|state| state.confirmed.get() == Some(true)));
            first.destroy();
            let rebuilt = gtk::ApplicationWindow::new(&application);
            bind(&rebuilt, &manager);
            manager.set_start_with_windows(false);
            pump_until(|| STATE.with(|state| state.confirmed.get() == Some(false)));
            assert_eq!(
                *backend.calls.lock().expect("calls"),
                vec![false, true, false]
            );
            second.destroy();
            rebuilt.destroy();
        },
    );
}

#[test]
fn registration_failure_restores_the_last_applied_preference() {
    crate::test_support::gtk_test(
        "ui::windows_desktop::startup::tests::registration_failure_restores_the_last_applied_preference",
        || {
            crate::ui::preferences::fixtures::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let backend = Arc::new(Fake {
                fail_enable: true,
                ..Fake::default()
            });
            BACKEND.with(|slot| slot.replace(backend.clone()));
            let application = application();
            let window = gtk::ApplicationWindow::new(&application);
            bind(&window, &manager);
            pump_until(|| STATE.with(|state| state.confirmed.get() == Some(false)));
            toggle_from_tray(&manager);
            pump_until(|| !manager.start_with_windows());
            assert!(backend.calls.lock().expect("calls").contains(&true));
            for window in application.windows() {
                window.destroy();
            }
        },
    );
}

#[test]
fn upgrading_old_settings_applies_enabled_startup_without_discarding_saved_options() {
    crate::test_support::gtk_test(
        "ui::windows_desktop::startup::tests::upgrading_old_settings_applies_enabled_startup_without_discarding_saved_options",
        || {
            crate::ui::preferences::fixtures::seed_saved_preferences_for_test();
            let path = crate::ui::preferences::config_directory().join("settings.toml");
            let mut saved: toml::Table = std::fs::read_to_string(&path)
                .expect("settings")
                .parse()
                .expect("TOML");
            saved.remove("start_with_windows");
            std::fs::write(path, toml::to_string(&saved).expect("old settings"))
                .expect("save old schema");
            let manager = PreferenceManager::shared();
            assert!(manager.start_with_windows());
            assert!(
                manager.keep_running_in_background(),
                "other saved options were discarded"
            );
            let backend = Arc::new(Fake::default());
            BACKEND.with(|slot| slot.replace(backend.clone()));
            let application = application();
            let window = gtk::ApplicationWindow::new(&application);
            bind(&window, &manager);
            pump_until(|| STATE.with(|state| state.confirmed.get() == Some(true)));
            assert_eq!(*backend.calls.lock().expect("calls"), vec![true]);
            window.destroy();
        },
    );
}

#[test]
fn rapid_startup_toggles_apply_the_latest_choice_after_an_inflight_registration() {
    crate::test_support::gtk_test(
        "ui::windows_desktop::startup::tests::rapid_startup_toggles_apply_the_latest_choice_after_an_inflight_registration",
        || {
            crate::ui::preferences::fixtures::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let (entered, ready) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let backend = Arc::new(Fake {
                hold: Mutex::new(Some((entered, wait))),
                ..Fake::default()
            });
            BACKEND.with(|slot| slot.replace(backend.clone()));
            let application = application();
            let window = gtk::ApplicationWindow::new(&application);
            bind(&window, &manager);
            pump_until(|| STATE.with(|state| state.confirmed.get() == Some(false)));
            manager.set_start_with_windows(true);
            ready
                .recv_timeout(Duration::from_secs(5))
                .expect("inflight enable");
            manager.set_start_with_windows(false);
            manager.set_start_with_windows(true);
            manager.set_start_with_windows(false);
            release.send(()).expect("finish inflight enable");
            pump_until(|| {
                STATE.with(|state| {
                    state.confirmed.get() == Some(false)
                        && state.confirmed_generation.get() == state.generation.get()
                })
            });
            assert!(!manager.start_with_windows());
            assert_eq!(
                *backend.calls.lock().expect("calls"),
                vec![false, true, false]
            );
            window.destroy();
        },
    );
}
