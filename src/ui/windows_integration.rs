// SPDX-License-Identifier: MIT
use super::preferences::PreferenceManager;
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    io,
    rc::Rc,
    sync::{Arc, mpsc},
};
use strata_win_shell::file_manager::{Actual, IntegrationStatus};

trait Backend: Send + Sync {
    fn apply(&self, work: &Work) -> io::Result<IntegrationStatus>;
    fn probe(&self) -> io::Result<IntegrationStatus>;
}
struct Native;
#[cfg(not(test))]
impl Backend for Native {
    fn apply(&self, work: &Work) -> io::Result<IntegrationStatus> {
        let executable = std::env::current_exe()?;
        if work.desired {
            strata_win_shell::file_manager::install_with_policy(
                &executable,
                &executable.with_file_name("strata-shell.exe"),
                &work.label,
                work.activation
                    .as_ref()
                    .map(|(scope, request)| (scope.as_str(), request.as_slice())),
                work.replace_foreign,
            )?;
        } else {
            strata_win_shell::file_manager::disable(&executable)?;
        }
        self.probe()
    }
    fn probe(&self) -> io::Result<IntegrationStatus> {
        strata_win_shell::file_manager::probe_status(Some(&std::env::current_exe()?))
    }
}

struct Work {
    desired: bool,
    replace_foreign: bool,
    write: bool,
    label: String,
    activation: Option<(String, Vec<u8>)>,
    backend: Arc<dyn Backend>,
    reply: async_channel::Sender<Outcome>,
}
struct Outcome {
    status: IntegrationStatus,
    failed: bool,
}
struct View {
    root: glib::WeakRef<gtk::Box>,
    label: glib::WeakRef<gtk::Label>,
    retry: glib::WeakRef<gtk::Button>,
}
#[derive(Default)]
struct State {
    desired: Cell<Option<bool>>,
    generation: Cell<u64>,
    pending: Cell<bool>,
    applying: Cell<bool>,
    refresh_again: Cell<bool>,
    outcome: RefCell<Option<Outcome>>,
    views: RefCell<Vec<View>>,
    sender: RefCell<Option<mpsc::Sender<Work>>>,
    #[cfg(not(test))]
    monitor: RefCell<Option<strata_win_shell::file_manager::RegistryMonitor>>,
}
thread_local! {
    static STATE: State = State::default();
    static BACKEND: RefCell<Arc<dyn Backend>> = RefCell::new(Arc::new(Native));
}

fn publish() {
    STATE.with(|state| {
        let outcome = state.outcome.borrow();
        let message = if state.pending.get() && (state.applying.get() || outcome.is_none()) {
            Some("Checking file manager integration…")
        } else if let Some(outcome) = outcome.as_ref() {
            if !state.desired.get().unwrap_or(false) {
                if outcome.failed {
                    Some("Strata has not restored the previous file manager.")
                } else {
                    None
                }
            } else {
                match outcome.status.actual {
                    Actual::Active if !outcome.failed => None,
                    Actual::Conflict => {
                        Some("Another application controls File Explorer integration.")
                    }
                    Actual::Unsupported => {
                        Some("This Windows version does not support this integration.")
                    }
                    _ => Some("Strata has not replaced File Explorer."),
                }
            }
        } else {
            None
        };
        state.views.borrow_mut().retain(|view| {
            let (Some(root), Some(label), Some(retry)) = (
                view.root.upgrade(),
                view.label.upgrade(),
                view.retry.upgrade(),
            ) else {
                return false;
            };
            root.set_visible(message.is_some());
            label.set_text(&message.map(crate::i18n::tr).unwrap_or_default());
            retry.set_sensitive(!state.pending.get());
            retry.set_visible(
                outcome
                    .as_ref()
                    .is_none_or(|outcome| outcome.status.actual != Actual::Unsupported),
            );
            true
        });
    });
}

fn sender() -> io::Result<mpsc::Sender<Work>> {
    STATE.with(|state| {
        if let Some(sender) = state.sender.borrow().as_ref() { return Ok(sender.clone()); }
        let (sender, receiver) = mpsc::channel::<Work>();
        std::thread::Builder::new().name("shell-integration".into()).spawn(move || {
            while let Ok(mut work) = receiver.recv() {
                for newer in receiver.try_iter() {work=newer;}
                let start = std::time::Instant::now();
                let result = if work.write {work.backend.apply(&work)} else {work.backend.probe()};
                let outcome = match result {
                    Ok(status) => Outcome { status, failed: false },
                    Err(error) => {
                        tracing::warn!(error_kind=?error.kind(), win32_error=error.raw_os_error(), desired=work.desired, "Windows shell reconciliation failed");
                        let mut status = work.backend.probe().unwrap_or_else(|_| IntegrationStatus::disabled());
                        if status.actual != Actual::Conflict {status.actual=Actual::Broken;}
                        Outcome { status, failed: true }
                    }
                };
                tracing::info!(desired=work.desired, write=work.write, actual=outcome.status.actual.as_str(), transaction_id=?outcome.status.transaction_id, elapsed_ms=start.elapsed().as_millis() as u64, "Windows shell integration checked");
                let _ = work.reply.send_blocking(outcome);
            }
        })?;
        state.sender.replace(Some(sender.clone()));
        Ok(sender)
    })
}

fn request(write: bool, replace_foreign: bool) {
    let Some((generation, desired)) = STATE.with(|state| {
        if !write && state.pending.get() {
            state.refresh_again.set(true);
            return None;
        }
        let generation = state.generation.get().wrapping_add(1);
        state.generation.set(generation);
        state.pending.set(true);
        state.applying.set(write);
        Some((generation, state.desired.get().unwrap_or(false)))
    }) else {
        return;
    };
    let (reply, receiver) = async_channel::bounded(1);
    let activation = super::windows_runtime::shell_activation().ok().flatten();
    let work = Work {
        desired,
        replace_foreign,
        write,
        label: crate::i18n::tr("Open with Strata"),
        activation,
        backend: BACKEND.with(|b| b.borrow().clone()),
        reply,
    };
    if sender()
        .and_then(|sender| sender.send(work).map_err(io::Error::other))
        .is_err()
    {
        STATE.with(|state| {
            state.pending.set(false);
            let mut status = IntegrationStatus::disabled();
            status.actual = Actual::Broken;
            state.outcome.replace(Some(Outcome {
                status,
                failed: true,
            }));
        });
        publish();
        return;
    }
    publish();
    glib::MainContext::default().spawn_local(async move {
        let Ok(mut outcome) = receiver.recv().await else {
            return;
        };
        let refresh = STATE.with(|state| {
            if state.generation.get() != generation {
                return false;
            }
            if !write
                && state
                    .outcome
                    .borrow()
                    .as_ref()
                    .is_some_and(|previous| previous.failed)
            {
                let satisfied = outcome.status.actual
                    == if desired {
                        Actual::Active
                    } else {
                        Actual::Disabled
                    };
                outcome.failed |= !satisfied;
                if outcome.failed && outcome.status.actual == Actual::Disabled {
                    outcome.status.actual = Actual::Broken;
                }
            }
            state.pending.set(false);
            state.outcome.replace(Some(outcome));
            state.refresh_again.replace(false)
        });
        publish();
        if refresh {
            request(false, false);
        }
    });
}

pub(super) fn refresh() {
    request(false, false);
}

pub(super) fn bind(window: &gtk::ApplicationWindow, manager: &Rc<PreferenceManager>) {
    manager.bind_preference(
        window,
        PreferenceManager::replace_file_explorer,
        move |_, enabled| {
            let changed = STATE.with(|state| state.desired.replace(Some(enabled)) != Some(enabled));
            request(changed, false);
        },
    );
    window.connect_is_active_notify(|window| {
        if window.is_active() {
            refresh();
        }
    });
    #[cfg(not(test))]
    STATE.with(|state| {
        if state.monitor.borrow().is_some() {
            return;
        }
        let (send, receive) = async_channel::bounded(1);
        if let Ok(monitor) = strata_win_shell::file_manager::watch_changes(move || {
            let _ = send.try_send(());
        }) {
            state.monitor.replace(Some(monitor));
            glib::MainContext::default().spawn_local(async move {
                while receive.recv().await.is_ok() {
                    refresh();
                }
            });
        }
    });
}

/// A single shared health row, shown only while unknown or degraded.
pub(super) fn status_row() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    root.add_css_class("settings-option");
    let label = gtk::Label::new(None);
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.add_css_class("settings-option-description");
    let retry = gtk::Button::with_label(&crate::i18n::tr("Try again"));
    retry.add_css_class("settings-action-button");
    retry.set_valign(gtk::Align::Center);
    root.append(&label);
    root.append(&retry);
    STATE.with(|state| {
        state.views.borrow_mut().push(View {
            root: root.downgrade(),
            label: label.downgrade(),
            retry: retry.downgrade(),
        })
    });
    retry.connect_clicked(|button| {
        let conflict=STATE.with(|state|state.desired.get()==Some(true) && state.outcome.borrow().as_ref().is_some_and(|outcome|outcome.status.actual==Actual::Conflict));
        if conflict {
            let Some(window)=button.root().and_downcast::<gtk::Window>() else {return};
            gtk::AlertDialog::builder().message(crate::i18n::tr("Replace the current file manager?"))
                .detail(crate::i18n::tr("Strata will save the current settings and restore them when you turn this option off."))
                .buttons([crate::i18n::tr("Cancel"),crate::i18n::tr("Replace")]).cancel_button(0).default_button(0).build()
                .choose(Some(&window),gio::Cancellable::NONE,|result| {if matches!(result,Ok(1)){request(true,true);}});
        } else {request(true,false);}
    });
    root.connect_map(|_| refresh());
    publish();
    root
}

#[cfg(test)]
mod tests;
