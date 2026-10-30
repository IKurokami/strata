// SPDX-License-Identifier: MIT
use super::*;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct Recording {
    calls: Mutex<Vec<bool>>,
    status: Mutex<IntegrationStatus>,
    unavailable: AtomicBool,
    hold: AtomicBool,
    entered: AtomicBool,
}
impl Recording {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            status: Mutex::new(IntegrationStatus::disabled()),
            unavailable: AtomicBool::new(false),
            hold: AtomicBool::new(false),
            entered: AtomicBool::new(false),
        })
    }
}
impl Backend for Recording {
    fn apply(&self, work: &Work) -> io::Result<IntegrationStatus> {
        assert!(
            !work.replace_foreign,
            "these paths must never authorize a takeover"
        );
        assert!(!work.label.is_empty());
        if let Some((scope, request)) = &work.activation {
            assert!(!scope.is_empty());
            assert!(!request.is_empty());
        }
        self.calls.lock().expect("calls").push(work.desired);
        if work.desired {
            self.entered.store(true, Ordering::Release);
            while self.hold.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        if self.unavailable.load(Ordering::Acquire) {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        let mut status = self.status.lock().expect("status");
        status.actual = if work.desired {
            Actual::Active
        } else {
            Actual::Disabled
        };
        Ok(status.clone())
    }
    fn probe(&self) -> io::Result<IntegrationStatus> {
        Ok(self.status.lock().expect("status").clone())
    }
}
fn until(ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ready() {
        assert!(Instant::now() < deadline, "integration worker timeout");
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn settle() {
    until(|| STATE.with(|state| !state.pending.get()));
}
fn application() -> gtk::Application {
    let application = gtk::Application::builder()
        .application_id("io.github.lgse.Strata.ShellStateTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(gio::Cancellable::NONE)
        .expect("register");
    application
}
fn last_retry() -> gtk::Button {
    STATE.with(|state| {
        state
            .views
            .borrow()
            .last()
            .expect("health view")
            .retry
            .upgrade()
            .expect("retry")
    })
}
fn actual() -> Actual {
    STATE.with(|state| {
        state
            .outcome
            .borrow()
            .as_ref()
            .expect("probe outcome")
            .status
            .actual
    })
}

#[test]
fn failed_registration_retains_intent_and_retry_recovers_health() {
    crate::test_support::gtk_test(
        "ui::windows_integration::tests::failed_registration_retains_intent_and_retry_recovers_health",
        || {
            let manager = PreferenceManager::shared();
            manager.set_replace_file_explorer(false);
            let backend = Recording::new();
            BACKEND.with(|current| current.replace(backend.clone()));
            let app = application();
            let window = gtk::ApplicationWindow::new(&app);
            bind(&window, &manager);
            settle();
            let row = status_row();
            window.set_child(Some(&row));
            window.present();
            settle();
            backend.unavailable.store(true, Ordering::Release);
            manager.set_replace_file_explorer(true);
            settle();
            assert!(
                manager.replace_file_explorer(),
                "a transient OS failure must retain user intent"
            );
            assert_eq!(actual(), Actual::Broken);
            assert!(row.is_visible());
            backend.unavailable.store(false, Ordering::Release);
            last_retry().emit_clicked();
            settle();
            assert_eq!(actual(), Actual::Active);
            assert!(!row.is_visible());
            assert!(manager.replace_file_explorer());
            backend.unavailable.store(true, Ordering::Release);
            manager.set_replace_file_explorer(false);
            settle();
            refresh();
            settle();
            assert!(!manager.replace_file_explorer());
            assert!(row.is_visible(), "a probe must not hide a failed restore");
            backend.unavailable.store(false, Ordering::Release);
            last_retry().emit_clicked();
            settle();
            assert_eq!(actual(), Actual::Disabled);
            assert!(!row.is_visible());
            window.destroy();
        },
    );
}

#[test]
fn saved_shell_preference_applies_before_settings_and_stays_live_across_windows() {
    crate::test_support::gtk_test(
        "ui::windows_integration::tests::saved_shell_preference_applies_before_settings_and_stays_live_across_windows",
        || {
            assert_eq!(
                strata_win_shell::file_manager::configuration_directory().unwrap(),
                crate::storage::config_directory()
            );
            crate::ui::preferences::fixtures::seed_saved_preferences_for_test();
            let manager = PreferenceManager::shared();
            let backend = Recording::new();
            BACKEND.with(|current| current.replace(backend.clone()));
            let app = application();
            let first = gtk::ApplicationWindow::new(&app);
            bind(&first, &manager);
            settle();
            assert_eq!(actual(), Actual::Active);
            let second = gtk::ApplicationWindow::new(&app);
            bind(&second, &manager);
            settle();
            assert_eq!(
                *backend.calls.lock().expect("calls"),
                vec![true],
                "opening another window probes without rewriting handlers"
            );
            let first_health = status_row();
            let second_health = status_row();
            assert!(!first_health.is_visible());
            assert!(!second_health.is_visible());
            manager.set_replace_file_explorer(false);
            settle();
            assert_eq!(actual(), Actual::Disabled);
            manager.set_replace_file_explorer(true);
            settle();
            assert_eq!(actual(), Actual::Active);
            backend.status.lock().expect("status").actual = Actual::Conflict;
            let before = backend.calls.lock().expect("calls").len();
            refresh();
            settle();
            assert_eq!(actual(), Actual::Conflict);
            assert!(manager.replace_file_explorer());
            assert_eq!(
                backend.calls.lock().expect("calls").len(),
                before,
                "health checks never reclaim another manager's ownership"
            );
            assert!(first_health.is_visible());
            assert!(second_health.is_visible());
            first.destroy();
            let rebuilt = gtk::ApplicationWindow::new(&app);
            bind(&rebuilt, &manager);
            settle();
            assert_eq!(actual(), Actual::Conflict);
            second.destroy();
            rebuilt.destroy();
        },
    );
}

#[test]
fn slow_registry_work_keeps_gtk_responsive_and_latest_choice_wins() {
    crate::test_support::gtk_test(
        "ui::windows_integration::tests::slow_registry_work_keeps_gtk_responsive_and_latest_choice_wins",
        || {
            let manager = PreferenceManager::shared();
            manager.set_replace_file_explorer(false);
            let backend = Recording::new();
            BACKEND.with(|current| current.replace(backend.clone()));
            let app = application();
            let window = gtk::ApplicationWindow::new(&app);
            bind(&window, &manager);
            settle();
            backend.hold.store(true, Ordering::Release);
            manager.set_replace_file_explorer(true);
            until(|| backend.entered.load(Ordering::Acquire));
            let painted = Rc::new(Cell::new(false));
            let idle = painted.clone();
            glib::idle_add_local_once(move || idle.set(true));
            until(|| painted.get());
            manager.set_replace_file_explorer(false);
            backend.hold.store(false, Ordering::Release);
            settle();
            assert!(!manager.replace_file_explorer());
            assert_eq!(actual(), Actual::Disabled);
            assert_eq!(backend.calls.lock().expect("calls").last(), Some(&false));
            window.destroy();
        },
    );
}

impl Backend for Native {
    fn apply(&self, _: &Work) -> io::Result<IntegrationStatus> {
        self.probe()
    }
    fn probe(&self) -> io::Result<IntegrationStatus> {
        Ok(IntegrationStatus::disabled())
    }
}
