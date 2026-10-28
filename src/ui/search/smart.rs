// SPDX-License-Identifier: MIT

use crate::services::{
    SearchItem,
    smart_search::{
        self, SearchSession,
        result::{SearchHit, SearchPhase},
    },
};
use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

type PreviewKey = (
    std::path::PathBuf,
    Option<(
        crate::services::smart_search::result::FileRevision,
        crate::services::smart_search::result::ContentLocator,
    )>,
);

pub(super) struct Panel {
    pub root: gtk::Box,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    list: gtk::ListView,
    names: RefCell<Vec<SearchItem>>,
    hits: RefCell<Vec<SearchHit>>,
    rows: RefCell<Vec<SearchItem>>,
    session: SearchSession,
    generation: Cell<u64>,
    submitted: Cell<std::time::Instant>,
    phase: Cell<Option<SearchPhase>>,
    mode: Cell<usize>,
    query: RefCell<String>,
    scope: RefCell<Option<std::path::PathBuf>>,
    kind: Cell<usize>,
    drawer: RefCell<Option<super::super::preview::PreviewDrawer>>,
    preview_slot: gtk::Box,
    preview_enabled: Cell<bool>,
    reconciling: Cell<bool>,
    preview_key: RefCell<Option<PreviewKey>>,
    status: gtk::Label,
}

impl Panel {
    pub fn new(activate: Rc<dyn Fn(SearchItem)>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        root.append(&controls);
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, object| {
            let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let row = gtk::Box::new(gtk::Orientation::Vertical, 3);
            row.add_css_class("search-result");
            for _ in 0..3 {
                let label = gtk::Label::new(None);
                label.set_xalign(0.0);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                row.append(&label);
            }
            item.set_child(Some(&row));
        });
        factory.connect_bind(|_, object| {
            let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(value) = item.item().and_downcast::<glib::BoxedAnyObject>() else {
                return;
            };
            let value = value.borrow::<SearchItem>();
            let Some(row) = item.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(title) = row.first_child().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(detail) = title.next_sibling().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(excerpt) = detail.next_sibling().and_downcast::<gtk::Label>() else {
                return;
            };
            title.set_text(&value.name);
            detail.set_text(&value.path.to_string_lossy());
            detail.add_css_class("search-result-path");
            if let Some(hit) = &value.content_hit {
                excerpt.set_text(&format!(
                    "{} · {}",
                    crate::i18n::tr(if hit.weak {
                        "Related match"
                    } else {
                        "Content match"
                    }),
                    if hit.excerpt.is_empty() {
                        hit.locator.anchor().unwrap_or("")
                    } else {
                        &hit.excerpt
                    }
                ));
            } else {
                excerpt.set_text(&crate::i18n::tr("Filename match"));
            }
        });
        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.set_single_click_activate(false);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        body.set_vexpand(true);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hexpand(true)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        body.append(&scroll);
        let preview_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        preview_slot.set_width_request(340);
        preview_slot.set_visible(false);
        body.append(&preview_slot);
        root.append(&body);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.add_css_class("search-status");
        root.append(&status);
        let state = Rc::new(Self {
            root,
            store,
            selection,
            list,
            names: RefCell::new(Vec::new()),
            hits: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            session: SearchSession::new(),
            generation: Cell::new(0),
            submitted: Cell::new(std::time::Instant::now()),
            phase: Cell::new(None),
            mode: Cell::new(0),
            query: RefCell::new(String::new()),
            scope: RefCell::new(None),
            kind: Cell::new(0),
            drawer: RefCell::new(None),
            preview_slot,
            preview_enabled: Cell::new(true),
            reconciling: Cell::new(false),
            preview_key: RefCell::new(None),
            status,
        });
        let (modes, buttons) =
            super::super::controls::segmented_control(&["All", "Name", "Content", "Smart"], 0);
        controls.append(&modes);
        let all = buttons[0].clone();
        super::super::preferences::PreferenceManager::shared().bind_preference(
            &buttons[3],
            super::super::preferences::PreferenceManager::smart_search_enabled,
            move |widget, enabled| {
                widget.set_sensitive(enabled);
                if !enabled {
                    all.set_active(true);
                }
            },
        );
        for (index, button) in buttons.into_iter().enumerate() {
            let weak = Rc::downgrade(&state);
            button.connect_toggled(move |button| {
                if button.is_active()
                    && let Some(state) = weak.upgrade()
                {
                    state.mode.set(index);
                    state.refresh_content();
                }
            });
        }
        let kinds = gtk::MenuButton::builder()
            .label(crate::i18n::tr("File type"))
            .build();
        let menu = gtk::Popover::new();
        menu.set_has_arrow(false);
        menu.add_css_class("column-popover");
        let choices = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for (index, label) in [
            "All",
            "Documents",
            "Images",
            "Audio",
            "Video",
            "Source code",
        ]
        .into_iter()
        .enumerate()
        {
            let (button, _) = super::super::controls::menu_option(&crate::i18n::tr(label), false);
            let weak = Rc::downgrade(&state);
            let popup = menu.downgrade();
            button.connect_clicked(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.kind.set(index);
                    state.refresh_content();
                }
                if let Some(menu) = popup.upgrade() {
                    menu.popdown();
                }
            });
            choices.append(&button);
        }
        menu.set_child(Some(&choices));
        kinds.set_popover(Some(&menu));
        controls.append(&kinds);
        kinds.add_css_class("column-menu-option");
        let preview = gtk::ToggleButton::with_label(&crate::i18n::tr("Preview"));
        preview.add_css_class("segmented-control-option");
        preview.set_active(true);
        let weak = Rc::downgrade(&state);
        preview.connect_toggled(move |button| {
            if let Some(state) = weak.upgrade() {
                state.preview_enabled.set(button.is_active());
                state.preview_selected();
            }
        });
        controls.append(&preview);
        let scope = gtk::MenuButton::builder()
            .label(crate::i18n::tr("Indexed folders"))
            .build();
        let popup = gtk::Popover::new();
        popup.set_has_arrow(false);
        popup.add_css_class("column-popover");
        let folders = gtk::Box::new(gtk::Orientation::Vertical, 0);
        popup.set_child(Some(&folders));
        scope.set_popover(Some(&popup));
        scope.add_css_class("column-menu-option");
        controls.append(&scope);
        let weak = Rc::downgrade(&state);
        let bound_popup = popup.downgrade();
        super::super::preferences::PreferenceManager::shared().bind_preference(
            &folders,
            super::super::preferences::PreferenceManager::smart_search_roots,
            move |widget, roots| {
                let Some(folders) = widget.downcast_ref::<gtk::Box>() else {
                    return;
                };
                while let Some(child) = folders.first_child() {
                    folders.remove(&child);
                }
                for root in std::iter::once(None).chain(roots.into_iter().map(Some)) {
                    let label = root.as_ref().map_or_else(
                        || crate::i18n::tr("All"),
                        |path| path.to_string_lossy().into_owned(),
                    );
                    let (button, _) = super::super::controls::menu_option(&label, false);
                    let weak = weak.clone();
                    let popup = bound_popup.clone();
                    button.connect_clicked(move |_| {
                        if let Some(state) = weak.upgrade() {
                            state.scope.replace(root.clone());
                            state.refresh_content();
                        }
                        if let Some(popup) = popup.upgrade() {
                            popup.popdown();
                        }
                    });
                    folders.append(&button);
                }
            },
        );
        let weak = Rc::downgrade(&state);
        super::super::preferences::PreferenceManager::shared().bind_preference(
            &state.root,
            |manager| smart_search::Config {
                enabled: manager.smart_search_enabled(),
                roots: manager.smart_search_roots(),
                include_code: manager.smart_search_include_code(),
                index_on_battery: manager.smart_search_index_on_battery(),
                paused: manager.smart_search_paused(),
            },
            move |_, config| {
                let enabled = config.enabled;
                smart_search::configure(config);
                if let Some(state) = weak.upgrade() {
                    if !enabled && state.mode.get() == 3 {
                        state.mode.set(0);
                    }
                    state.refresh_content();
                }
            },
        );
        let weak = Rc::downgrade(&state);
        state.selection.connect_selected_notify(move |_| {
            if let Some(state) = weak.upgrade() {
                state.preview_selected();
            }
        });
        let weak = Rc::downgrade(&state);
        state.list.connect_activate(move |_, position| {
            if let Some(state) = weak.upgrade()
                && let Some(item) = state.rows.borrow().get(position as usize).cloned()
            {
                activate(item);
            }
        });
        let weak = Rc::downgrade(&state);
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            for response in state.session.take() {
                if !crate::services::smart_search::result::accepts_phase(
                    state.generation.get(),
                    state.phase.get(),
                    &response,
                ) {
                    continue;
                }
                state.phase.set(Some(response.phase));
                state.hits.replace(response.hits);
                state.render();
            }
            glib::ControlFlow::Continue
        });
        state
    }

    pub fn query(&self, query: &str) {
        self.submitted.set(std::time::Instant::now());
        self.query.replace(query.to_owned());
        self.names.borrow_mut().clear();
        self.refresh_content();
    }
    fn refresh_content(&self) {
        self.phase.set(None);
        self.hits.borrow_mut().clear();
        if self
            .scope
            .borrow()
            .as_ref()
            .is_some_and(|root| !smart_search::configured_roots().contains(root))
        {
            self.scope.take();
        }
        let roots = self
            .scope
            .borrow()
            .clone()
            .map_or_else(smart_search::configured_roots, |root| vec![root]);
        self.generation.set(self.session.query(
            &self.query.borrow(),
            roots,
            self.mode.get(),
            smart_search::result::SearchKind::from_index(self.kind.get()),
        ));
        self.render();
    }
    pub fn names(&self, items: Vec<SearchItem>) {
        self.names.replace(items);
        self.render();
    }
    pub fn selected(&self) -> Option<SearchItem> {
        self.rows
            .borrow()
            .get(self.selection.selected() as usize)
            .cloned()
    }
    pub fn move_selection(&self, direction: i32) {
        let count = self.rows.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let selected = self.selection.selected();
        let next = if selected == gtk::INVALID_LIST_POSITION {
            0
        } else {
            (selected as i32 + direction).clamp(0, count - 1)
        };
        self.selection.set_selected(next as u32);
        self.list
            .scroll_to(next as u32, gtk::ListScrollFlags::FOCUS, None);
    }
    pub fn cancel(&self) {
        self.session.cancel();
        self.preview_key.take();
        if let Some(drawer) = self.drawer.borrow().as_ref() {
            drawer.close();
        }
        self.preview_slot.set_visible(false);
    }
    fn render(&self) {
        let selected = self.selected();
        let mut rows = Vec::new();
        if self.mode.get() <= 1 {
            rows.extend(self.names.borrow().iter().cloned());
        }
        if self.mode.get() != 1 {
            rows.extend(
                self.hits
                    .borrow()
                    .iter()
                    .cloned()
                    .map(|hit| SearchItem::for_hit(Arc::new(hit))),
            );
        }
        rows.retain(|row| {
            smart_search::result::SearchKind::from_index(self.kind.get()).accepts(&row.path)
        });
        rows.truncate(200);
        self.status.set_text(&crate::i18n::tr(if rows.is_empty() {
            "No results"
        } else if self.phase.get() == Some(SearchPhase::Semantic) {
            "Local semantic and keyword results"
        } else {
            "Filename and keyword results"
        }));
        if *self.rows.borrow() == rows {
            return;
        }
        let old = self.rows.borrow();
        let prefix = old.iter().zip(&rows).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(rows[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let additions: Vec<_> = rows[prefix..rows.len() - suffix]
            .iter()
            .cloned()
            .map(glib::BoxedAnyObject::new)
            .collect();
        let removed = old.len() - prefix - suffix;
        drop(old);
        self.rows.replace(rows);
        self.reconciling.set(true);
        self.store.splice(prefix as u32, removed as u32, &additions);
        let retained = selected.and_then(|selected| {
            self.rows.borrow().iter().position(|row| {
                row.path == selected.path
                    && row.content_hit.as_ref().map(|hit| hit.identity())
                        == selected.content_hit.as_ref().map(|hit| hit.identity())
            })
        });
        self.selection
            .set_selected(retained.map_or(gtk::INVALID_LIST_POSITION, |position| position as u32));
        self.reconciling.set(false);
        self.preview_selected();
        tracing::debug!(generation=self.generation.get(),rows=self.rows.borrow().len(),phase=?self.phase.get(),elapsed_micros=self.submitted.get().elapsed().as_micros(),"search results published to virtual list");
        if tracing::enabled!(tracing::Level::DEBUG) {
            let submitted = self.submitted.get();
            let generation = self.generation.get();
            self.list.add_tick_callback(move |_, _| {
                tracing::debug!(
                    generation,
                    elapsed_micros = submitted.elapsed().as_micros(),
                    "search result frame ready"
                );
                glib::ControlFlow::Break
            });
        }
        self.status
            .set_text(&crate::i18n::tr(if self.rows.borrow().is_empty() {
                "No results"
            } else if self.phase.get() == Some(SearchPhase::Semantic) {
                "Local semantic and keyword results"
            } else {
                "Filename and keyword results"
            }));
    }
    fn preview_selected(&self) {
        if self.reconciling.get() {
            return;
        }
        let selected = self.selected();
        let entry = selected
            .as_ref()
            .map(SearchItem::preview_entry)
            .filter(super::super::preview::entry_supports_quick_preview);
        if !self.preview_enabled.get() || entry.is_none() {
            self.preview_key.take();
            if let Some(drawer) = self.drawer.borrow().as_ref() {
                drawer.close();
            }
            self.preview_slot.set_visible(false);
            return;
        }
        let key = selected.as_ref().map(|item| {
            (
                item.path.clone(),
                item.content_hit
                    .as_ref()
                    .map(|hit| (hit.revision.clone(), hit.locator.clone())),
            )
        });
        if *self.preview_key.borrow() == key && self.preview_slot.is_visible() {
            return;
        }
        self.preview_key.replace(key);
        if self.drawer.borrow().is_none() {
            let provider = crate::adapters::LocalPreviewProvider::new(Rc::new(|| {
                super::super::preferences::PreferenceManager::shared().media_preview_backend()
            }));
            let drawer = super::super::preview::PreviewDrawer::new(Rc::new(provider), true);
            let widget = drawer.widget();
            let weak = self.preview_slot.downgrade();
            widget.connect_visible_notify(move |widget| {
                if let Some(slot) = weak.upgrade() {
                    slot.set_visible(widget.is_visible());
                }
            });
            self.preview_slot.append(&widget);
            self.drawer.replace(Some(drawer));
        }
        self.preview_slot.set_visible(true);
        if let (Some(drawer), Some(entry)) = (self.drawer.borrow().as_ref(), entry) {
            if let Some(hit) = selected.and_then(|item| item.content_hit) {
                drawer.show_hit(hit, None);
            } else {
                drawer.show(entry, None);
            }
        }
    }
}

#[cfg(all(test, not(windows)))]
mod tests;
