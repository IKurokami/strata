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
    collections::HashMap,
    path::Path,
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
    query: RefCell<String>,
    drawer: RefCell<Option<super::super::preview::PreviewDrawer>>,
    preview_slot: gtk::Box,
    results_column: gtk::Overlay,
    reconciling: Cell<bool>,
    preview_key: RefCell<Option<PreviewKey>>,
    status: gtk::Label,
}

impl Panel {
    pub fn new(activate: Rc<dyn Fn(SearchItem)>) -> Rc<Self> {

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("search-smart-panel");
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, object| {
            let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("search-smart-result");
            let icon = crate::assets::primary_icon(crate::assets::icons::DOCUMENTS, 20);
            icon.set_valign(gtk::Align::Center);
            row.append(&icon);
            let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
            labels.set_hexpand(true);
            for class in ["search-result-name", "search-result-path", "search-smart-excerpt"] {
                let label = gtk::Label::new(None);
                label.set_xalign(0.0);
                label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                label.add_css_class(class);
                labels.append(&label);
            }
            row.append(&labels);
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
            let Some(icon) = row.first_child().and_downcast::<gtk::Image>() else {
                return;
            };
            let Some(labels) = icon.next_sibling().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(title) = labels.first_child().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(detail) = title.next_sibling().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(excerpt) = detail.next_sibling().and_downcast::<gtk::Label>() else {
                return;
            };
            crate::assets::set_primary_icon(
                &icon,
                if value.is_directory {
                    crate::assets::icons::FOLDER
                } else {
                    crate::ui::browser::icon_for_name(&value.name)
                },
            );
            title.set_text(&value.name);
            let visible_path = display_search_path(&value.path);
            detail.set_text(&visible_path);
            let snippet = value
                .content_hit
                .as_ref()
                .map(|hit| hit.excerpt.split_whitespace().take(32).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            excerpt.set_text(&snippet);
            excerpt.set_visible(!snippet.is_empty());
            crate::ui::accessibility::set_label(&row, &visible_path);
        });
        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("search-smart-list");
        list.set_single_click_activate(false);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hexpand(true)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let results_column = gtk::Overlay::new();
        results_column.set_hexpand(true);
        results_column.set_vexpand(true);
        results_column.set_child(Some(&scroll));
        let status = gtk::Label::new(Some(&crate::i18n::tr("No results")));
        status.add_css_class("search-smart-empty");
        status.set_halign(gtk::Align::Center);
        status.set_valign(gtk::Align::Center);
        status.set_visible(false);
        results_column.add_overlay(&status);
        let preview_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        preview_slot.add_css_class("search-smart-preview");
        preview_slot.set_hexpand(true);
        preview_slot.set_vexpand(true);
        preview_slot.set_overflow(gtk::Overflow::Hidden);
        preview_slot.set_visible(false);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.set_vexpand(true);
        body.set_hexpand(true);
        body.append(&results_column);
        body.append(&preview_slot);
        root.append(&body);
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
            query: RefCell::new(String::new()),
            drawer: RefCell::new(None),
            preview_slot,
            results_column,
            reconciling: Cell::new(false),
            preview_key: RefCell::new(None),
            status,
        });
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
                smart_search::configure(config);
                if let Some(state) = weak.upgrade() {
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
        self.generation.set(self.session.query(
            &self.query.borrow(),
            smart_search::configured_roots(),
            0,
            smart_search::result::SearchKind::Any,
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
    pub fn activation_target(&self) -> Option<SearchItem> {
        self.selected()
            .or_else(|| self.rows.borrow().first().cloned())
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
        let selected_key = self.selected().map(|item| result_key(&item.path));
        let rows = unique_rows(&self.names.borrow(), &self.hits.borrow());
        self.status.set_visible(rows.is_empty());
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
        let retained = selected_key.and_then(|key| {
            self.rows.borrow().iter().position(|row| result_key(&row.path) == key)
        });
        self.selection
            .set_selected(retained.map_or(gtk::INVALID_LIST_POSITION, |position| position as u32));
        self.reconciling.set(false);
        self.preview_selected();
        tracing::debug!(
            generation = self.generation.get(),
            rows = self.rows.borrow().len(),
            phase = ?self.phase.get(),
            elapsed_micros = self.submitted.get().elapsed().as_micros(),
            "search results published to virtual list"
        );
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
        if entry.is_none() {
            self.results_column.set_width_request(-1);
            self.results_column.set_hexpand(true);
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
        self.results_column.set_width_request(375);
        self.results_column.set_hexpand(false);
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

pub(super) fn display_search_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let raw = raw.as_ref();
    for (prefix, replacement) in [
        (r"\\?\UNC\", r"\\"),
        ("//?/UNC/", "//"),
        (r"\\?\", ""),
        ("//?/", ""),
    ] {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return format!("{replacement}{rest}");
        }
    }
    raw.to_owned()
}

fn result_key(path: &Path) -> String {
    let visible = display_search_path(path);
    if cfg!(windows) {
        visible.to_lowercase()
    } else {
        visible
    }
}

fn unique_rows(names: &[SearchItem], hits: &[SearchHit]) -> Vec<SearchItem> {
    let mut positions: HashMap<String, usize> = HashMap::with_capacity(names.len() + hits.len());
    let mut rows = Vec::with_capacity(names.len() + hits.len());
    for item in names {
        let key = result_key(&item.path);
        if !positions.contains_key(&key) {
            positions.insert(key, rows.len());
            rows.push(item.clone());
        }
    }
    for hit in hits {
        let item = SearchItem::for_hit(Arc::new(hit.clone()));
        let key = result_key(&item.path);
        if let Some(&position) = positions.get(&key) {
            let replace = rows[position].content_hit.as_ref().is_none_or(|previous| {
                (previous.weak && !hit.weak)
                    || (previous.weak == hit.weak && hit.score > previous.score)
            });
            if replace {
                rows[position] = item;
            }
        } else {
            positions.insert(key, rows.len());
            rows.push(item);
        }
    }
    rows.truncate(200);
    rows
}

#[cfg(all(test, not(windows)))]
mod tests;
