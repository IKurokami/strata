// SPDX-License-Identifier: MIT

use super::{browser::BrowserView, preferences::PreferenceManager};
use gtk::{gio, glib, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    rc::Rc,
    time::Duration,
};
use strata_win_shell::activation::{self, Server};

#[derive(Serialize, Deserialize)]
enum Request {
    Open {
        cwd: Vec<u16>,
        arguments: Vec<Vec<u16>>,
    },
    Quit,
    Unlock {
        operand: String,
    },
}

fn activation_scope() -> String {
    format!(
        "io.github.lgse.Strata|{}",
        glib::user_config_dir().display()
    )
    .to_lowercase()
}

pub(super) fn shell_activation() -> std::io::Result<Option<(String, Vec<u8>)>> {
    if !AVAILABLE.with(Cell::get) {
        return Ok(None);
    }
    let cwd = std::env::current_dir()?;
    let request = Request::Open {
        cwd: cwd.as_os_str().encode_wide().collect(),
        arguments: Vec::new(),
    };
    let message = serde_json::to_vec(&request).map_err(std::io::Error::other)?;
    Ok(Some((activation_scope(), message)))
}

/// One primary per configuration, current user and logon session, across installations.
pub(crate) fn claim(arguments: &[OsString]) -> Result<Option<Server>, glib::ExitCode> {
    let request = if arguments.len() == 2 && arguments[1] == "--quit" {
        Request::Quit
    } else if arguments.len() == 3 && arguments[1] == "--unlock-volume" {
        Request::Unlock {
            operand: arguments[2].to_string_lossy().into_owned(),
        }
    } else if arguments.len() == 2
        && matches!(
            arguments[1].to_str(),
            Some("--help" | "-h" | "--help-all" | "--help-gapplication")
        )
    {
        return Ok(None);
    } else {
        let mut literal = false;
        let mut files = Vec::new();
        for argument in arguments.iter().skip(1) {
            if !literal && argument == "--" {
                literal = true;
                continue;
            }
            if !literal && argument.to_string_lossy().starts_with('-') {
                if argument == "--gapplication-service" {
                    continue;
                }
                eprintln!(
                    "Unsupported Windows application option: {}",
                    argument.to_string_lossy()
                );
                return Err(glib::ExitCode::FAILURE);
            }
            files.push(argument.encode_wide().collect());
        }
        if files.len() > 256 {
            return Err(glib::ExitCode::FAILURE);
        }
        let Ok(cwd) = std::env::current_dir() else {
            return Err(glib::ExitCode::FAILURE);
        };
        Request::Open {
            cwd: cwd.as_os_str().encode_wide().collect(),
            arguments: files,
        }
    };
    let scope = activation_scope();
    let Ok(message) = serde_json::to_vec(&request) else {
        return Err(glib::ExitCode::FAILURE);
    };
    let forwarded = activation::forward(&scope, &message);
    if forwarded.is_ok() {
        return Err(glib::ExitCode::SUCCESS);
    }
    if matches!(request, Request::Quit) {
        return Err(
            if forwarded
                .err()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                glib::ExitCode::SUCCESS
            } else {
                glib::ExitCode::FAILURE
            },
        );
    }
    match Server::bind(&scope) {
        Ok(server) => Ok(Some(server)),
        Err(error) => {
            // A simultaneous first launch may have claimed the endpoint just after our probe.
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                if activation::forward(&scope, &message).is_ok() {
                    return Err(glib::ExitCode::SUCCESS);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            eprintln!("Windows activation unavailable: {error}");
            Err(glib::ExitCode::FAILURE)
        }
    }
}

pub(crate) fn install(application: &gtk::Application, server: Server) {
    AVAILABLE.with(|available| available.set(true));
    let weak = application.downgrade();
    let timer = Rc::new(RefCell::new(None));
    let timer_for_startup = timer.clone();
    let server = Rc::new(server);
    application.connect_startup(move |_| {
        let weak = weak.clone();
        let server = server.clone();
        timer_for_startup.replace(Some(glib::timeout_add_local(
            Duration::from_millis(16),
            move || {
                let Some(application) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                for _ in 0..8 {
                    let Some(message) = server.receive() else {
                        break;
                    };
                    if let Err(error) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            dispatch(&application, &message)
                        }))
                    {
                        tracing::error!("Windows activation failed: {error:?}");
                    }
                }
                glib::ControlFlow::Continue
            },
        )));
    });
    application.connect_shutdown(move |_| {
        AVAILABLE.with(|available| available.set(false));
        if let Some(timer) = timer.take() {
            timer.remove();
        }
        let cached = HIDDEN.with(|hidden| hidden.borrow_mut().take());
        drop(cached);
    });
}

fn dispatch(application: &gtk::Application, message: &[u8]) {
    let Ok(request) = serde_json::from_slice::<Request>(message) else {
        tracing::warn!("Invalid Windows activation request");
        return;
    };
    match request {
        Request::Quit => quit(application),
        Request::Unlock { operand } => {
            if let Ok(target) = crate::UnlockTarget::parse(&operand) {
                super::present_unlock(application, target);
            }
        }
        Request::Open { cwd, arguments } => {
            if arguments.len() > 256
                || cwd.contains(&0)
                || arguments.iter().any(|argument| argument.contains(&0))
            {
                return;
            }
            let cwd = OsString::from_wide(&cwd);
            if !std::path::Path::new(&cwd).is_absolute() {
                return;
            }
            if arguments.is_empty() {
                super::present(application);
            }
            for argument in arguments {
                let file =
                    gio::File::for_commandline_arg_and_cwd(OsString::from_wide(&argument), &cwd);
                super::present_open(application, file);
            }
        }
    }
}

thread_local! {
    static AVAILABLE: Cell<bool> = const { Cell::new(false) };
    static HIDDEN: RefCell<Option<(glib::WeakRef<gtk::ApplicationWindow>, BrowserView)>> = const { RefCell::new(None) };
}

pub(super) fn present_window(window: &impl IsA<gtk::Window>) {
    window.present();
    if let Some(window) = window.as_ref().downcast_ref::<gtk::ApplicationWindow>()
        && let Some(handle) = super::windows_window_state::handle(window)
    {
        strata_win_shell::window::foreground(handle);
    }
}

pub(super) fn retain(window: &gtk::ApplicationWindow, browser: BrowserView) -> bool {
    if !AVAILABLE.with(Cell::get) {
        return false;
    }
    let retained = HIDDEN.with(|hidden| {
        let mut hidden = hidden.borrow_mut();
        if hidden.as_ref().is_some_and(|(window, _)| {
            window
                .upgrade()
                .is_some_and(|window| window.application().is_some())
        }) {
            return false;
        }
        *hidden = Some((window.downgrade(), browser));
        true
    });
    if retained {
        window.set_visible(false);
    }
    retained
}

pub(super) fn reuse(application: &gtk::Application) -> Option<BrowserView> {
    let cached = HIDDEN.with(|hidden| {
        let belongs = hidden.borrow().as_ref().is_some_and(|(window, _)| {
            window
                .upgrade()
                .is_some_and(|window| window.application().as_ref() == Some(application))
        });
        if belongs {
            hidden.borrow_mut().take()
        } else {
            None
        }
    });
    let (window, browser) = cached?;
    let window = window.upgrade()?;
    if window.application().as_ref() != Some(application) {
        return None;
    }
    let started = std::time::Instant::now();
    present_window(&window);
    browser.restore_listing_focus();
    tracing::info!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        "resident window reused"
    );
    Some(browser)
}

pub(super) fn present_existing(application: &gtk::Application) -> bool {
    if reuse(application).is_some() {
        return true;
    }
    let window = application
        .active_window()
        .filter(|window| window.is_visible())
        .or_else(|| {
            application
                .windows()
                .into_iter()
                .find(|window| window.is_visible() && window.is::<gtk::ApplicationWindow>())
        });
    let Some(window) = window else {
        return false;
    };
    present_window(&window);
    true
}

pub(super) fn bind(window: &gtk::ApplicationWindow, preferences: &Rc<PreferenceManager>) {
    super::windows_desktop::bind(window, preferences);
    preferences.bind_preference(
        window,
        PreferenceManager::keep_running_in_background,
        move |_, enabled| {
            if !enabled {
                let cached = HIDDEN.with(|hidden| hidden.borrow_mut().take());
                if let Some((window, _)) = cached
                    && let Some(window) = window.upgrade()
                {
                    window.destroy();
                }
            }
        },
    );
}

pub(super) fn quit(application: &gtk::Application) {
    if let Some((window, blocker)) = super::close_guard::application_blocker(application) {
        window.present();
        blocker.show(&window);
        return;
    }
    for window in application.windows() {
        // Explicit exit bypasses close_request's Win32 IME teardown ordering.
        gtk::prelude::RootExt::set_focus(&window, None::<&gtk::Widget>);
        window.destroy();
    }
    application.quit();
}

#[cfg(test)]
mod tests;
