// SPDX-License-Identifier: MIT

use super::preferences::PreferenceManager;
#[cfg(not(test))]
use gtk::glib;
use gtk::prelude::*;
#[cfg(not(test))]
use std::time::Duration;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use strata_win_shell::desktop::Tray;
mod startup;
#[cfg(not(test))]
use strata_win_shell::desktop::{Console, Event};

thread_local! {
    #[cfg(not(test))]
    static CONSOLE: RefCell<Console> = RefCell::new(Console::default());
    static APPLIED: Cell<Option<bool>> = const { Cell::new(None) };
    #[cfg(test)]
    static CALLS: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
    static BACKGROUND: RefCell<Option<(gtk::ApplicationWindow, gtk::gio::ApplicationHoldGuard, Rc<PreferenceManager>)>> = const { RefCell::new(None) };
}

pub(super) fn bind(window: &gtk::ApplicationWindow, manager: &Rc<PreferenceManager>) {
    #[cfg(not(test))]
    if let Some(application) = window.application() {
        initialize(&application, manager);
    }
    startup::bind(window, manager);
    #[cfg(not(test))]
    let weak = Rc::downgrade(manager);
    manager.bind_preference(
        window,
        PreferenceManager::show_console,
        move |_, enabled| {
            if APPLIED.with(|state| state.get() == Some(enabled)) {
                return;
            }
            #[cfg(not(test))]
            if let Err(error) = CONSOLE.with(|console| console.borrow_mut().set_visible(enabled)) {
                tracing::warn!(%error, "unable to show application console");
                if let Some(manager) = weak.upgrade() {
                    manager.set_show_console(false);
                }
                return;
            }
            #[cfg(test)]
            CALLS.with(|calls| calls.borrow_mut().push(enabled));
            APPLIED.with(|state| state.set(Some(enabled)));
            TRAY.with(|tray| {
                if let Some(tray) = tray.borrow().as_ref() {
                    tray.set_console_checked(enabled);
                }
            });
        },
    );
}

pub(super) fn toggle_startup(manager: &PreferenceManager) {
    startup::toggle_from_tray(manager);
}

#[cfg(test)]
mod tests;

#[cfg(not(test))]
fn initialize(application: &gtk::Application, manager: &Rc<PreferenceManager>) {
    if TRAY.with(|tray| tray.borrow().is_some()) {
        return;
    }
    let Some(icon) = tray_icon() else { return };
    let tray = match Tray::with_startup_action(
        &crate::i18n::tr("Open Strata"),
        &crate::i18n::tr("Show application console"),
        &crate::i18n::tr("Quit Strata"),
        &crate::i18n::tr("Clear preview cache"),
        &crate::i18n::tr("Start with Windows"),
        &icon,
    ) {
        Ok(tray) => tray,
        Err(error) => {
            tracing::warn!(%error, "unable to create system tray icon");
            return;
        }
    };
    drop(TRAY.with(|slot| slot.replace(Some(tray))));
    let weak = application.downgrade();
    let manager = Rc::downgrade(manager);
    let timer = Rc::new(RefCell::new(Some(glib::timeout_add_local(
        Duration::from_millis(32),
        move || {
            let Some(application) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let event = TRAY.with(|slot| slot.borrow().as_ref().and_then(Tray::receive));
            match event {
                Some(Event::Open) => {
                    if !super::windows_runtime::present_existing(&application) {
                        super::present(&application);
                    }
                }
                Some(Event::Console) => {
                    if let Some(manager) = manager.upgrade() {
                        manager.set_show_console(!manager.show_console());
                    }
                }
                Some(Event::Startup) => {
                    if let Some(manager) = manager.upgrade() {
                        toggle_startup(&manager);
                    }
                }
                Some(Event::Quit) => super::windows_runtime::quit(&application),
                Some(Event::ClearCache) => crate::ui::cache_control::clear(|_| {}),
                None | Some(Event::Restart) => {}
            }
            glib::ControlFlow::Continue
        },
    ))));
    application.connect_shutdown(move |_| {
        if let Some(timer) = timer.take() {
            timer.remove();
        }
        drop(TRAY.with(|slot| slot.borrow_mut().take()));
        BACKGROUND.with(|slot| {
            if let Some((anchor, hold, _manager)) = slot.take() {
                anchor.destroy();
                drop(hold);
            }
        });
    });
}

pub(crate) fn start_background(application: &gtk::Application) {
    if !application.windows().is_empty() || BACKGROUND.with(|slot| slot.borrow().is_some()) {
        return;
    }
    let manager = PreferenceManager::shared();
    #[cfg(not(test))]
    {
        initialize(application, &manager);
        if TRAY.with(|slot| slot.borrow().is_none()) {
            return;
        }
    }
    // This preference anchor is never attached, realized or presented as an app window.
    let anchor = gtk::ApplicationWindow::builder().build();
    bind(&anchor, &manager);
    BACKGROUND.with(|slot| slot.replace(Some((anchor, application.hold(), manager))));
    application.connect_window_added(|_, _| {
        BACKGROUND.with(|slot| {
            if let Some((anchor, hold, _manager)) = slot.take() {
                anchor.destroy();
                drop(hold);
            }
        });
    });
}

#[cfg(not(test))]
fn tray_icon() -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(
        include_str!("../../data/brand/strata-logo-white.svg"),
        &usvg::Options::default(),
    )
    .ok()?;
    let mut pixels = tiny_skia::Pixmap::new(32, 32)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(32.0 / tree.size().width(), 32.0 / tree.size().height()),
        &mut pixels.as_mut(),
    );
    pixels.encode_png().ok()
}
