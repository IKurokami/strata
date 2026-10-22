// SPDX-License-Identifier: MIT

use super::{browser::BrowserView, preferences::PreferenceManager};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use strata_win_shell::shell_window::{Event, Registration};

pub(super) fn bind(
    window: &gtk::ApplicationWindow,
    preferences: &Rc<PreferenceManager>,
    browser: impl Fn() -> Option<BrowserView> + 'static,
    reveal: impl Fn(PathBuf, Vec<PathBuf>) + 'static,
) {
    let enabled = Rc::new(Cell::new(false));
    let registration: Rc<RefCell<Option<(PathBuf, Registration)>>> = Rc::new(RefCell::new(None));
    let enabled_binding = enabled.clone();
    let registered = registration.clone();
    preferences.bind_preference(
        window,
        PreferenceManager::replace_file_explorer,
        move |_, active| {
            enabled_binding.set(active);
            if !active {
                registered.borrow_mut().take();
            }
        },
    );
    let weak = window.downgrade();
    let registered = registration.clone();
    let mut retry_after = Instant::now();
    let timer = Rc::new(RefCell::new(Some(glib::timeout_add_local(
        Duration::from_millis(16),
        move || {
            let Some(window) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !enabled.get() {
                registered.borrow_mut().take();
                return glib::ControlFlow::Continue;
            }
            let Some(browser) = browser() else {
                return glib::ControlFlow::Continue;
            };
            let Some(folder) = browser
                .browser()
                .active_location()
                .and_then(|location| location.native_path().map(PathBuf::from))
            else {
                registered.borrow_mut().take();
                return glib::ControlFlow::Continue;
            };
            let mut slot = registered.borrow_mut();
            if let Some((previous, registration)) = slot.as_mut() {
                if *previous != folder {
                    registration.navigate(folder.clone());
                    *previous = folder;
                }
            } else if Instant::now() >= retry_after
                && let Some(handle) = super::windows_window_state::handle(&window)
            {
                match Registration::new(handle, folder.clone()) {
                    Ok(registration) => *slot = Some((folder, registration)),
                    Err(error) => {
                        retry_after = Instant::now() + Duration::from_secs(2);
                        tracing::warn!(%error, "unable to start Windows shell registration");
                    }
                }
            }
            let mut selection = None;
            let mut show = false;
            let mut failed = false;
            if let Some((_, registration)) = slot.as_ref() {
                for _ in 0..64 {
                    match registration.receive() {
                        Some(Event::Registered) => {
                            tracing::debug!("Windows shell window registered")
                        }
                        Some(Event::Selection { folder, files }) => {
                            selection = Some((folder, files))
                        }
                        Some(Event::Show) => show = true,
                        Some(Event::Failed(error)) => {
                            failed = true;
                            tracing::warn!(%error, "Windows shell window registration failed");
                        }
                        None => break,
                    }
                }
            }
            if failed {
                slot.take();
                retry_after = Instant::now() + Duration::from_secs(2);
            }
            drop(slot);
            if let Some((folder, files)) = selection
                && !files.is_empty()
            {
                tracing::debug!(selected = files.len(), "Windows shell selection received");
                reveal(folder, files);
                show = true;
            }
            if show {
                super::windows_runtime::present_window(&window);
            }
            glib::ControlFlow::Continue
        },
    ))));
    window.connect_destroy(move |_| {
        if let Some(timer) = timer.borrow_mut().take() {
            timer.remove();
        }
        registration.borrow_mut().take();
    });
}
