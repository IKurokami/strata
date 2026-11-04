// SPDX-License-Identifier: MIT

use super::*;
use gtk::glib;
use std::{
    io,
    sync::{Arc, mpsc},
};

trait Backend: Send + Sync {
    fn apply(&self, enabled: bool) -> io::Result<()>;
}

struct Native;
impl Backend for Native {
    fn apply(&self, enabled: bool) -> io::Result<()> {
        #[cfg(not(test))]
        if std::env::var_os("STRATA_DISABLE_AUTOSTART").is_none() {
            strata_win_shell::file_manager::startup::set_enabled(
                &std::env::current_exe()?,
                enabled,
            )?;
        }
        #[cfg(test)]
        let _ = enabled;
        Ok(())
    }
}

struct Work {
    enabled: bool,
    backend: Arc<dyn Backend>,
    reply: async_channel::Sender<io::Result<()>>,
}

#[derive(Default)]
struct State {
    desired: Cell<Option<bool>>,
    confirmed: Cell<Option<bool>>,
    confirmed_generation: Cell<u64>,
    generation: Cell<u64>,
    sender: RefCell<Option<mpsc::Sender<Work>>>,
}

thread_local! {
    static STATE: State = State::default();
    static BACKEND: RefCell<Arc<dyn Backend>> = RefCell::new(Arc::new(Native));
}

fn queue(work: Work) -> io::Result<()> {
    STATE.with(|state| {
        if state.sender.borrow().is_none() {
            let (sender, receiver) = mpsc::channel::<Work>();
            std::thread::Builder::new()
                .name("windows-startup".into())
                .spawn(move || {
                    while let Ok(mut work) = receiver.recv() {
                        for newer in receiver.try_iter() {
                            work = newer;
                        }
                        let result = work.backend.apply(work.enabled);
                        let _ = work.reply.send_blocking(result);
                    }
                })?;
            state.sender.replace(Some(sender));
        }
        state
            .sender
            .borrow()
            .as_ref()
            .expect("startup worker")
            .send(work)
            .map_err(io::Error::other)
    })
}

pub(super) fn toggle_from_tray(manager: &PreferenceManager) {
    manager.set_start_with_windows(!manager.start_with_windows());
}

pub(super) fn bind(window: &gtk::ApplicationWindow, manager: &Rc<PreferenceManager>) {
    let weak = Rc::downgrade(manager);
    manager.bind_preference(
        window,
        PreferenceManager::start_with_windows,
        move |anchor, enabled| {
            TRAY.with(|tray| {
                if let Some(tray) = tray.borrow().as_ref() {
                    tray.set_startup_checked(enabled);
                }
            });
            let generation = STATE.with(|state| {
                if state.desired.replace(Some(enabled)) == Some(enabled) {
                    return None;
                }
                let generation = state.generation.get().wrapping_add(1);
                state.generation.set(generation);
                Some(generation)
            });
            let Some(generation) = generation else {
                return;
            };
            let Some(manager) = weak.upgrade() else {
                return;
            };
            let anchor = anchor.downgrade();
            let (send, receive) = async_channel::bounded(1);
            let queued = queue(Work {
                enabled,
                backend: BACKEND.with(|backend| backend.borrow().clone()),
                reply: send,
            });
            glib::MainContext::default().spawn_local(async move {
                let result = match queued {
                    Ok(()) => match receive.recv().await {
                        Ok(result) => result,
                        Err(_) => return,
                    },
                    Err(error) => Err(error),
                };
                if result.is_ok() {
                    STATE.with(|state| {
                        if generation >= state.confirmed_generation.get() {
                            state.confirmed.set(Some(enabled));
                            state.confirmed_generation.set(generation);
                        }
                    });
                }
                if STATE.with(|state| state.generation.get() != generation) {
                    return;
                }
                if let Err(error) = result {
                    tracing::warn!(%error, "unable to change Windows startup settings");
                    let previous = STATE.with(|state| state.confirmed.get().unwrap_or(false));
                    manager.set_start_with_windows(previous);
                    if let Some(anchor) = anchor.upgrade() {
                        crate::ui::modal::show_error_dialog(
                            &anchor,
                            &crate::i18n::tr("Unable to change Windows startup settings"),
                            &crate::services::io_error_message(&error),
                        );
                    }
                }
            });
        },
    );
}

#[cfg(test)]
mod tests;
