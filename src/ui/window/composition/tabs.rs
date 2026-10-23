// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::location_for_file,
    model::Location,
    ui::{
        blur::BlurBin,
        browser::BrowserView,
        keys::{self, KeyPress, bindings::*},
        preferences::PreferenceManager,
        tabs_session,
    },
};

use super::{WindowContent, layout, tenxer_splash};

mod strip;
use strip::TabStrip;

#[cfg(test)]
mod tests;

struct Tab {
    id: u64,
    content: Rc<WindowContent>,
    focus: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    pending_location: RefCell<Option<Location>>,
    #[cfg(windows)]
    requested_location: RefCell<Option<Location>>,
}

#[cfg(windows)]
thread_local! {
    static WINDOWS: RefCell<Vec<std::rc::Weak<TabWindow>>> = const { RefCell::new(Vec::new()) };
}

#[cfg(windows)]
pub(in crate::ui::window) fn existing(application: &gtk::Application) -> Option<Rc<TabWindow>> {
    WINDOWS.with_borrow_mut(|windows| {
        windows.retain(|window| window.strong_count() != 0);
        windows
            .iter()
            .filter_map(std::rc::Weak::upgrade)
            .find(|tabs| {
                !tabs.tabs.borrow().is_empty()
                    && tabs
                        .window
                        .upgrade()
                        .is_some_and(|window| window.application().as_ref() == Some(application))
            })
    })
}

pub(in crate::ui::window) struct TabWindow {
    window: glib::WeakRef<gtk::ApplicationWindow>,
    preferences: Rc<PreferenceManager>,
    stack: gtk::Stack,
    drag_token: String,
    strip: TabStrip,
    tabs: RefCell<Vec<Rc<Tab>>>,
    active: Cell<u64>,
    next_id: Cell<u64>,
    hints: Cell<bool>,
    restoring: Cell<bool>,
    persist: bool,
}

impl TabWindow {
    pub(in crate::ui::window) fn new(
        window: &gtk::ApplicationWindow,
        preferences: &Rc<PreferenceManager>,
        persist: bool,
    ) -> Rc<Self> {
        let stack = gtk::Stack::new();
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::None);
        let strip = TabStrip::new();
        let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
        shell.append(strip.widget());
        shell.append(&stack);
        let overlay = gtk::Overlay::new();
        overlay.add_css_class("tab-window");
        overlay.set_child(Some(&BlurBin::new(&shell)));
        window.set_child(Some(&overlay));
        let state = Rc::new(Self {
            window: window.downgrade(),
            preferences: preferences.clone(),
            stack,
            strip,
            tabs: RefCell::new(Vec::new()),
            active: Cell::new(0),
            next_id: Cell::new(1),
            hints: Cell::new(false),
            restoring: Cell::new(false),
            persist,
            drag_token: glib::uuid_string_random().to_string(),
        });
        state.add(None);
        #[cfg(windows)]
        WINDOWS.with_borrow_mut(|windows| windows.push(Rc::downgrade(&state)));
        state.install_actions(window);
        state.install_keys(window);
        #[cfg(windows)]
        window.connect_close_request(|window| {
            // GTK 4.24's Win32 IME filter is attached to the focused widget.
            // Clear focus before window teardown so queued IME messages cannot
            // reach a disposed GtkIMContextIME instance.
            gtk::prelude::RootExt::set_focus(window, None::<&gtk::Widget>);
            glib::Propagation::Proceed
        });
        super::super::install_modal_focus_trap(window);
        let weak = Rc::downgrade(&state);
        crate::ui::modal::set_modal_focus_fallback(
            window.upcast_ref(),
            Rc::new(move || {
                let browser = weak.upgrade().and_then(|state| {
                    let active = state.active.get();
                    state
                        .tabs
                        .borrow()
                        .iter()
                        .find(|tab| tab.id == active)
                        .map(|tab| tab.content.browser.clone())
                });
                if let Some(browser) = browser {
                    browser.restore_listing_focus();
                }
            }),
        );
        tenxer_splash::install(window, &overlay, preferences);
        preferences.register_save_notice_window(window.upcast_ref());
        let weak = Rc::downgrade(&state);
        crate::ui::close_guard::install(window, move |_| {
            let state = weak.upgrade()?;
            state
                .tabs
                .borrow()
                .iter()
                .any(|tab| tab.content.browser.browser().has_background_operations())
                .then(operations_active)
        });
        #[cfg(windows)]
        {
            crate::ui::windows_runtime::bind(window, preferences);
            let shell_state = Rc::downgrade(&state);
            let reveal_state = Rc::downgrade(&state);
            crate::ui::windows_shell::bind(
                window,
                preferences,
                move || {
                    let state = shell_state.upgrade()?;
                    let active = state.active.get();
                    let browser = state
                        .tabs
                        .borrow()
                        .iter()
                        .find(|tab| tab.id == active)
                        .map(|tab| tab.content.browser.clone());
                    browser
                },
                move |folder, files| {
                    if let Some(state) = reveal_state.upgrade() {
                        state.present_external(
                            Some(Location::local(folder)),
                            files.into_iter().map(Location::local).collect(),
                            false,
                            true,
                        );
                    }
                },
            );
            let weak = Rc::downgrade(&state);
            window.connect_close_request(move |window| {
                let Some(state) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if !state.preferences.keep_running_in_background()
                    || crate::ui::window::visible_modal_layer(window).is_some()
                {
                    return glib::Propagation::Proceed;
                }
                for tab in state.tabs.borrow().iter() {
                    tab.content.preview.clear_target();
                    tab.content.footer.shortcuts.cancel_chord();
                    tab.content.footer.shortcuts.dismiss_prompt();
                    tab.content.browser.browser().close_peek();
                }
                state.persist_session();
                if crate::ui::windows_runtime::retain(window, state.active_browser()) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
        }
        let retained = state.clone();
        window.connect_unrealize(move |window| {
            PreferenceManager::shared().release_bindings_within(window);
            for tab in retained.tabs.take() {
                tab.content.dispose();
            }
            for name in [
                "search",
                "jump-folder",
                "refresh",
                "open-terminal",
                "toggle-arrow-scope",
                "new-tab",
                "close-tab",
                "previous-tab",
                "select-tab",
                "open-tab-at",
                "open-window-at",
            ] {
                window.remove_action(name);
            }
        });
        let weak = Rc::downgrade(&state);
        window.connect_is_active_notify(move |window| {
            if !window.is_active()
                && let Some(state) = weak.upgrade()
            {
                state.show_hints(false);
            }
        });
        state
    }

    fn active_tab(&self) -> Rc<Tab> {
        self.tabs
            .borrow()
            .iter()
            .find(|tab| tab.id == self.active.get())
            .expect("a window always has an active tab")
            .clone()
    }

    pub(in crate::ui::window) fn active_browser(&self) -> BrowserView {
        self.active_tab().content.browser.clone()
    }

    #[cfg(windows)]
    pub(in crate::ui::window) fn present_external(
        self: &Rc<Self>,
        location: Option<Location>,
        selection: Vec<Location>,
        properties: bool,
        navigate: bool,
    ) -> BrowserView {
        let Some(location) = location else {
            if let Some(window) = self.window.upgrade() {
                crate::ui::windows_runtime::present_window(&window);
            }
            return self.active_browser();
        };
        let existing = self
            .tabs
            .borrow()
            .iter()
            .find(|tab| {
                tab.content.browser.browser().active_location().as_ref() == Some(&location)
                    || tab.pending_location.borrow().as_ref() == Some(&location)
                    || tab.requested_location.borrow().as_ref() == Some(&location)
            })
            .cloned();
        let tab = existing.unwrap_or_else(|| {
            self.add_with_location(None, false);
            self.tabs.borrow().last().expect("new external tab").clone()
        });
        *tab.requested_location.borrow_mut() = Some(location.clone());
        let browser = tab.content.browser.clone();
        if navigate {
            tab.pending_location.take();
            if selection.is_empty() {
                if browser.browser().active_location().as_ref() != Some(&location) {
                    browser.navigate_location(location);
                }
            } else {
                browser.reveal_external_locations(location, selection, properties);
            }
        }
        self.select(tab.id);
        if self.blocked() {
            let weak = Rc::downgrade(self);
            let id = tab.id;
            glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                let Some(state) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if !state.tabs.borrow().iter().any(|tab| tab.id == id) {
                    return glib::ControlFlow::Break;
                }
                if state.blocked() {
                    return glib::ControlFlow::Continue;
                }
                state.select(id);
                if let Some(window) = state.window.upgrade() {
                    crate::ui::windows_runtime::present_window(&window);
                }
                glib::ControlFlow::Break
            });
        }
        if let Some(window) = self.window.upgrade() {
            crate::ui::windows_runtime::present_window(&window);
        }
        browser
    }

    fn blocked(&self) -> bool {
        self.window
            .upgrade()
            .is_none_or(|window| super::super::visible_modal_layer(&window).is_some())
    }

    fn new_tab(self: &Rc<Self>) {
        if self.blocked() {
            return;
        }
        let location = self.active_browser().browser().active_location();
        self.add(location.or_else(|| Some(super::super::startup_location(&self.preferences))));
    }

    fn open_in_new_tab(self: &Rc<Self>, location: Location) {
        if self.blocked() || self.window.upgrade().is_none() {
            return;
        }
        tracing::debug!(location = %location.display_path(), "opening location in new tab");
        self.add(None);
        super::super::open_argument::route_location(self.active_browser(), location);
    }

    fn open_in_new_window(self: &Rc<Self>, location: Location) -> Option<BrowserView> {
        let application = self.window.upgrade()?.application()?;
        tracing::debug!(location = %location.display_path(), "opening location in new window");
        Some(super::super::open_argument::present_location_window(
            &application,
            location,
        ))
    }

    fn add(self: &Rc<Self>, location: Option<Location>) {
        self.add_with_location(location, true);
    }

    fn add_prepared(self: &Rc<Self>, location: Location) {
        self.add_with_location(Some(location), false);
    }

    fn add_with_location(self: &Rc<Self>, location: Option<Location>, navigate: bool) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let content = Rc::new(WindowContent::new(&window, &self.preferences));
        content.bind_context(&window, &self.preferences);
        let tab = Rc::new(Tab {
            id,
            content: content.clone(),
            focus: RefCell::new(None),
            pending_location: RefCell::new((!navigate).then_some(location.clone()).flatten()),
            #[cfg(windows)]
            requested_location: RefCell::new(None),
        });
        self.stack
            .add_named(&content.overlay, Some(&id.to_string()));
        let weak_tab = Rc::downgrade(&tab);
        let weak = Rc::downgrade(self);
        content.header.new_tab.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.new_tab();
            }
        });
        self.tabs.borrow_mut().push(tab);
        self.strip.add(self, id, &content.browser);
        if let Some(location) = location.as_ref().filter(|_| !navigate) {
            self.strip.label(id, &tab_label(Some(location)));
        }
        let weak = Rc::downgrade(self);
        content.browser.observe_tab_location(move |location| {
            if let Some(state) = weak.upgrade() {
                state.strip.label(id, &tab_label(location));
                state.persist_session();
            }
            if location.is_some()
                && let Some(tab) = weak_tab.upgrade()
            {
                tab.pending_location.take();
                #[cfg(windows)]
                tab.requested_location.take();
            }
        });
        if id == 1 {
            super::super::schedule_after_first_paint(&window, &content.sidebar, &self.preferences);
        } else {
            content.sidebar.schedule_after_first_paint(&content.overlay);
        }
        if navigate {
            self.select(id);
        } else {
            self.refresh_chrome();
        }
        self.refresh_chrome();
        if navigate && let Some(location) = location {
            content.browser.navigate_location(location);
        }
    }

    fn select(&self, id: u64) {
        if self.active.get() == id || self.blocked() {
            return;
        }
        let Some(tab) = self.tabs.borrow().iter().find(|tab| tab.id == id).cloned() else {
            return;
        };
        let Some(window) = self.window.upgrade() else {
            return;
        };
        if self.active.get() != 0 {
            let previous = self.active_tab();
            *previous.focus.borrow_mut() =
                gtk::prelude::RootExt::focus(&window).map(|widget| widget.downgrade());
            #[cfg(windows)]
            keys::release_native_text_focus(&previous.content.overlay);
            previous.content.footer.shortcuts.cancel_chord();
            previous.content.browser.cancel_location_edit();
        }
        self.active.set(id);
        self.stack.set_visible_child(&tab.content.overlay);
        tab.content.activate_actions(&window);
        let pending_location = tab.pending_location.borrow().clone();
        self.refresh_chrome();
        self.strip.select(id);
        let saved = tab
            .focus
            .borrow()
            .as_ref()
            .and_then(glib::WeakRef::upgrade)
            .filter(|widget| widget.is_mapped());
        // Saved focus may be a pane surface that is no longer focusable.
        if !saved.is_some_and(|focus| focus.grab_focus()) {
            tab.content.browser.browser().focus_active();
        }
        if let Some(location) = pending_location {
            tab.content.browser.navigate_location(location);
            if tab.content.browser.browser().active_location().is_some() {
                tab.pending_location.take();
            }
        }
        self.persist_session();
    }

    fn close(&self, id: u64) {
        if self.blocked() {
            return;
        }
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let tabs = self.tabs.borrow();
        let Some(index) = tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        if tabs[index]
            .content
            .browser
            .browser()
            .has_background_operations()
        {
            operations_active().show(window.upcast_ref());
            return;
        }
        if tabs.len() == 1 {
            drop(tabs);
            window.close();
            return;
        }
        let tab = tabs[index].clone();
        let next = tabs[if index + 1 < tabs.len() {
            index + 1
        } else {
            index - 1
        }]
        .id;
        drop(tabs);
        if self.active.get() == id {
            self.select(next);
        }
        self.tabs.borrow_mut().retain(|tab| tab.id != id);
        tab.content.dispose();
        self.stack.remove(&tab.content.overlay);
        self.strip.remove(id);
        self.refresh_chrome();
        self.persist_session();
    }

    fn refresh_chrome(&self) {
        let multiple = self.tabs.borrow().len() > 1;
        self.strip.widget().set_visible(multiple);
        for tab in self.tabs.borrow().iter() {
            let header = &tab.content.header;
            if let Some(parent) = header.new_tab.parent().and_downcast::<gtk::Box>() {
                parent.remove(&header.new_tab);
            }
            if let Some(parent) = header.close.parent().and_downcast::<gtk::Box>() {
                parent.remove(&header.close);
            }
            if multiple && tab.id == self.active.get() {
                self.strip.actions.append(&header.new_tab);
                self.strip.end.append(&header.close);
            } else {
                header.actions.prepend(&header.new_tab);
                header.actions.append(&header.close);
                header.new_tab.set_visible(!multiple);
                header
                    .close
                    .set_visible(!multiple && self.preferences.window_show_close());
            }
            if tab.id == self.active.get() {
                header.new_tab.set_visible(true);
                header
                    .close
                    .set_visible(self.preferences.window_show_close());
            }
        }
        self.strip.hints(self.hints.get());
    }

    fn select_index(&self, index: usize) {
        let id = self.tabs.borrow().get(index).map(|tab| tab.id);
        if let Some(id) = id {
            self.select(id);
        }
    }

    fn install_actions(self: &Rc<Self>, window: &gtk::ApplicationWindow) {
        let create = gio::SimpleAction::new("new-tab", None);
        let weak = Rc::downgrade(self);
        create.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.new_tab();
            }
        });
        window.add_action(&create);

        let close = gio::SimpleAction::new("close-tab", None);
        let weak = Rc::downgrade(self);
        close.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.close(state.active.get());
            }
        });
        window.add_action(&close);

        let previous = gio::SimpleAction::new("previous-tab", None);
        let weak = Rc::downgrade(self);
        previous.connect_activate(move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.cycle(-1);
            }
        });
        window.add_action(&previous);

        let select = gio::SimpleAction::new("select-tab", Some(&u32::static_variant_type()));
        let weak = Rc::downgrade(self);
        select.connect_activate(move |_, parameter| {
            if let Some(state) = weak.upgrade()
                && let Some(index) = parameter.and_then(|value| value.get::<u32>())
            {
                state.select_index(index as usize);
            }
        });
        window.add_action(&select);

        // Window actions let menu rows and gestures reach this `TabWindow`.
        let open_tab = gio::SimpleAction::new("open-tab-at", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        open_tab.connect_activate(move |_, parameter| {
            if let (Some(state), Some(location)) = (weak.upgrade(), action_location(parameter)) {
                state.open_in_new_tab(location);
            }
        });
        window.add_action(&open_tab);

        let open_window =
            gio::SimpleAction::new("open-window-at", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        open_window.connect_activate(move |_, parameter| {
            if let (Some(state), Some(location)) = (weak.upgrade(), action_location(parameter)) {
                state.open_in_new_window(location);
            }
        });
        window.add_action(&open_window);
    }

    fn cycle(&self, delta: i32) {
        let tabs = self.tabs.borrow();
        let current = tabs
            .iter()
            .position(|tab| tab.id == self.active.get())
            .unwrap_or(0);
        let index = (current as i32 + delta).rem_euclid(tabs.len() as i32) as usize;
        let id = tabs[index].id;
        drop(tabs);
        self.select(id);
    }

    fn reorder(&self, source: u64, target: u64) {
        if source == target || self.blocked() {
            return;
        }
        let mut tabs = self.tabs.borrow_mut();
        let Some(from) = tabs.iter().position(|tab| tab.id == source) else {
            return;
        };
        let Some(to) = tabs.iter().position(|tab| tab.id == target) else {
            return;
        };
        let tab = tabs.remove(from);
        tabs.insert(to, tab);
        self.strip
            .reorder(&tabs.iter().map(|tab| tab.id).collect::<Vec<_>>());
        self.strip.hints(self.hints.get());
        drop(tabs);
        self.persist_session();
    }

    fn move_active_tab(&self, delta: i32) {
        let tabs = self.tabs.borrow();
        let Some(current) = tabs.iter().position(|tab| tab.id == self.active.get()) else {
            return;
        };
        let target = current
            .checked_add_signed(delta as isize)
            .and_then(|index| tabs.get(index))
            .map(|tab| tab.id);
        drop(tabs);
        if let Some(target) = target {
            self.reorder(self.active.get(), target);
        }
    }

    fn show_hints(&self, show: bool) {
        if self.hints.replace(show) != show {
            self.strip.hints(show);
        }
    }

    fn persist_session(&self) {
        if !self.persist || self.restoring.get() {
            return;
        }
        if !self.preferences.restore_tabs() {
            tabs_session::remove();
            return;
        }
        let tabs = self.tabs.borrow();
        let mut locations = Vec::with_capacity(tabs.len());
        let mut active = 0;
        for tab in tabs.iter() {
            let Some(location) = tab
                .content
                .browser
                .browser()
                .active_location()
                .or_else(|| tab.pending_location.borrow().clone())
            else {
                continue;
            };
            if tab.id == self.active.get() {
                active = locations.len();
            }
            locations.push(location);
        }
        drop(tabs);
        if locations.is_empty() {
            return;
        }
        tabs_session::save(&locations, active);
    }

    pub(in crate::ui::window) fn try_restore(self: &Rc<Self>) -> bool {
        if !self.preferences.restore_tabs() {
            return false;
        }
        let Some(session) = tabs_session::load_restorable() else {
            return false;
        };
        if session.tabs.is_empty() {
            return false;
        }
        self.restore_session(session);
        true
    }

    #[cfg(any(windows, test))]
    pub(in crate::ui::window) fn restore_when_ready(
        self: &Rc<Self>,
        loading: impl std::future::Future<Output = tabs_session::StartupSession> + 'static,
    ) -> glib::JoinHandle<()> {
        let state = self.clone();
        let browser = self.active_browser();
        let generation = browser.browser().navigation_generation();
        let active = self.active.get();
        let next_id = self.next_id.get();
        glib::MainContext::default().spawn_local(async move {
            let mut loaded = loading.await;
            let unchanged = || {
                browser.overlay().root().is_some()
                    && state
                        .window
                        .upgrade()
                        .is_some_and(|window| window.application().is_some())
                    && !state.tabs.borrow().is_empty()
                    && browser.browser().navigation_generation() == generation
                    && state.active.get() == active
                    && state.next_id.get() == next_id
            };
            if !unchanged() {
                return;
            }
            while state.blocked() {
                glib::timeout_future(std::time::Duration::from_millis(25)).await;
                if !unchanged() {
                    return;
                }
            }
            if loaded.session.is_some() && !state.preferences.restore_tabs() {
                let default = state.preferences.default_directory();
                loaded =
                    gio::spawn_blocking(move || tabs_session::StartupSession::fallback(default))
                        .await
                        .unwrap_or_else(|_| tabs_session::StartupSession::fallback(None));
            }
            if !unchanged() {
                return;
            }
            if let Some(invalid) = loaded.invalid_default
                && state.preferences.default_directory().as_ref() == Some(&invalid)
            {
                state.preferences.set_default_directory(None);
            }
            if let Some(session) = loaded.session {
                state.restore_session(session);
            } else if let Some(fallback) = loaded.fallback {
                browser.navigate_location(Location::local(fallback));
            }
        })
    }

    fn restore_session(self: &Rc<Self>, session: tabs_session::RestoredSession) {
        if session.tabs.is_empty() || self.blocked() {
            return;
        }
        self.restoring.set(true);
        let active_index = session.active.min(session.tabs.len() - 1);
        if active_index == 0 {
            self.active_browser()
                .navigate_location(session.tabs[0].clone());
        } else {
            self.prepare_tab(self.active.get(), session.tabs[0].clone());
        }
        for (index, location) in session.tabs.iter().enumerate().skip(1) {
            if index == active_index {
                self.add(Some(location.clone()));
            } else {
                self.add_prepared(location.clone());
            }
        }
        self.select_index(active_index);
        self.restoring.set(false);
    }

    fn prepare_tab(&self, id: u64, location: Location) {
        let Some(tab) = self.tabs.borrow().iter().find(|tab| tab.id == id).cloned() else {
            return;
        };
        tab.pending_location.replace(Some(location.clone()));
        self.strip.label(id, &tab_label(Some(&location)));
    }

    fn handle_key(self: &Rc<Self>, press: &KeyPress) -> glib::Propagation {
        // GDK reports a modifier key's state from before its own press.
        self.show_hints((press.modifiers() | press.modifier_key()) == TAB_NUMBERS.modifiers());
        if self.blocked() || !is_tab_shortcut(press) {
            return glib::Propagation::Proceed;
        }
        self.active_tab().content.footer.shortcuts.cancel_chord();
        if press.is(TAB_NEW) {
            self.new_tab();
        } else if press.is(TAB_CLOSE) {
            self.close(self.active.get());
        } else if press.is_any(&[TAB_NEXT, TAB_NEXT_PAGE_DOWN]) {
            self.cycle(1);
        } else if press.is_any(&[TAB_PREVIOUS, TAB_PREVIOUS_PAGE_UP]) {
            self.cycle(-1);
        } else if press.is(TAB_MOVE_LEFT) {
            self.move_active_tab(-1);
        } else if press.is(TAB_MOVE_RIGHT) {
            self.move_active_tab(1);
        } else if let Some(digit) = tab_number(press) {
            self.select_index(tab_index(digit));
        }
        glib::Propagation::Stop
    }

    fn install_keys(self: &Rc<Self>, window: &gtk::ApplicationWindow) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys::on_press(&keys, move |_, press| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |state| state.handle_key(press))
        });
        let weak = Rc::downgrade(self);
        keys::on_release(&keys, move |_, press| {
            if let Some(state) = weak.upgrade() {
                state.show_hints(
                    press.modifiers().difference(press.modifier_key()) == TAB_NUMBERS.modifiers(),
                );
            }
        });
        window.add_controller(keys);
    }
}

fn operations_active() -> crate::ui::close_guard::CloseBlocker {
    crate::ui::close_guard::CloseBlocker {
        title: crate::i18n::tr("File operations are still active"),
        detail: crate::i18n::tr(
            "Wait for these operations to finish, or cancel them before closing this tab or window. Cancellation does not undo completed changes.",
        ),
    }
}

pub(in crate::ui::window) fn is_tab_shortcut(press: &KeyPress) -> bool {
    press.is_any(&[
        TAB_NEW,
        TAB_CLOSE,
        TAB_NEXT,
        TAB_PREVIOUS,
        TAB_NEXT_PAGE_DOWN,
        TAB_PREVIOUS_PAGE_UP,
        TAB_MOVE_LEFT,
        TAB_MOVE_RIGHT,
    ]) || tab_number(press).is_some()
}

/// A tab digit, unless the key types the `+` or `−` of a text size shortcut,
/// as the 1 key does with Shift on Swiss German.
fn tab_number(press: &KeyPress) -> Option<u8> {
    if press.is_any(&[TEXT_LARGER, TEXT_SMALLER]) {
        return None;
    }
    press.digit_in(TAB_NUMBERS)
}

fn action_location(parameter: Option<&glib::Variant>) -> Option<Location> {
    let uri = parameter?.get::<String>()?;
    location_for_file(&gio::File::for_uri(&uri))
}

/// The tab a digit selects: 1 is the first tab and 0 the tenth.
pub(in crate::ui::window) fn tab_index(digit: u8) -> usize {
    (usize::from(digit) + 9) % 10
}

fn tab_label(location: Option<&Location>) -> String {
    let Some(location) = location else {
        return "Home".into();
    };
    if let Some(path) = location.native_path() {
        if Some(path)
            == std::env::var_os("HOME")
                .as_deref()
                .map(std::path::Path::new)
        {
            return "~".into();
        }
        return path
            .file_name()
            .map_or_else(|| "/".into(), |name| name.to_string_lossy().into_owned());
    }
    let uri = location.uri_value().unwrap_or_default();
    match uri {
        "trash:///" => "Trash".into(),
        "recent:///" => "Recent".into(),
        "network:///" => "Network".into(),
        _ => gio::File::for_uri(uri).basename().map_or_else(
            || "Location".into(),
            |name| name.to_string_lossy().into_owned(),
        ),
    }
}
