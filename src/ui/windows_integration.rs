// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    io,
    rc::Rc,
};

#[cfg(test)]
use gtk::prelude::*;

use super::preferences::PreferenceManager;

trait Backend {
    fn apply(&self, enabled: bool) -> io::Result<()>;
}

#[cfg(not(test))]
struct Native;

#[cfg(not(test))]
impl Backend for Native {
    fn apply(&self, enabled: bool) -> io::Result<()> {
        let executable = std::env::current_exe()?;
        if enabled {
            let source = executable.with_file_name("strata-shell.exe");
            let activation = super::windows_runtime::shell_activation()?;
            let result = strata_win_shell::file_manager::install(
                &executable,
                &source,
                &crate::i18n::tr("Open with Strata"),
                activation
                    .as_ref()
                    .map(|(scope, request)| (scope.as_str(), request.as_slice())),
            );
            if result.is_err() {
                strata_win_shell::file_manager::disable(&executable)?;
            }
            result
        } else {
            strata_win_shell::file_manager::disable(&executable)
        }
    }
}

#[cfg(test)]
struct Native;

#[cfg(test)]
impl Backend for Native {
    fn apply(&self, _: bool) -> io::Result<()> {
        Ok(())
    }
}

thread_local! {
    static APPLIED: Cell<Option<bool>> = const { Cell::new(None) };
    static BACKEND: RefCell<Rc<dyn Backend>> = RefCell::new(Rc::new(Native));
}

pub(super) fn bind(window: &gtk::ApplicationWindow, manager: &Rc<PreferenceManager>) {
    let weak = Rc::downgrade(manager);
    manager.bind_preference(
        window,
        PreferenceManager::replace_file_explorer,
        move |window, enabled| {
            let previous = APPLIED.with(|state| state.get());
            if previous == Some(enabled) {
                return;
            }
            let result = BACKEND.with(|backend| backend.borrow().apply(enabled));
            if let Err(error) = result {
                tracing::warn!(%error, enabled, "unable to apply Windows file-manager integration");
                let previous = previous.unwrap_or(false);
                APPLIED.with(|state| state.set(Some(previous)));
                if let Some(manager) = weak.upgrade() {
                    manager.set_replace_file_explorer(previous);
                }
                let help = crate::i18n::tr(
                    "Restart Strata and try again. If the problem persists, reinstall the application.",
                );
                super::modal::show_error_dialog(
                    window,
                    &crate::i18n::tr("Unable to change the default file manager"),
                    &help,
                );
            } else {
                APPLIED.with(|state| state.set(Some(enabled)));
            }
        },
    );
}

#[cfg(test)]
mod tests;
