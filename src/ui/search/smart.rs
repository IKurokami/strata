// SPDX-License-Identifier: MIT

use crate::services::{
    SearchItem,
    smart_search::{
        self, SearchSession,
        result::{SearchHit, SearchPhase},
    },
};
use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::Path,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SearchSlotLayout;

    #[glib::object_subclass]
    impl ObjectSubclass for SearchSlotLayout {
        const NAME: &'static str = "StrataSearchSlotLayout";
        type Type = super::SearchSlotLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for SearchSlotLayout {}

    impl LayoutManagerImpl for SearchSlotLayout {
        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            _for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let mut child = widget.first_child();
            while let Some(current) = child {
                if current.should_layout() {
                    let _ = current.measure(orientation, -1);
                }
                child = current.next_sibling();
            }
            (0, 0, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, _baseline: i32) {
            let mut child = widget.first_child();
            while let Some(current) = child {
                if current.should_layout() {
                    current.size_allocate(
                        &gtk::Allocation::new(0, 0, width.max(0), height.max(0)),
                        -1,
                    );
                }
                child = current.next_sibling();
            }
        }
    }
}

glib::wrapper! {
    pub struct SearchSlotLayout(ObjectSubclass<imp::SearchSlotLayout>)
        @extends gtk::LayoutManager;
}

impl Default for SearchSlotLayout {
    fn default() -> Self {
        glib::Object::new()
    }
}

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

            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("search-smart-result");
            let thumbnail = super::super::thumbnail::ThumbnailSlot::new(36);
            thumbnail.add_css_class("search-smart-thumbnail");
            thumbnail.set_valign(gtk::Align::Center);
            row.append(&thumbnail);
            let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
            labels.set_hexpand(true);
            let heading = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let title = gtk::Label::new(None);
            title.set_xalign(0.0);
            title.set_hexpand(true);
            title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            title.add_css_class("search-result-name");
            heading.append(&title);
            let place = gtk::Label::new(None);
            place.add_css_class("search-smart-place");
            place.set_valign(gtk::Align::Center);
            heading.append(&place);
            labels.append(&heading);
            let detail = gtk::Label::new(None);
            detail.set_xalign(0.0);
            detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
            detail.add_css_class("search-smart-secondary");
            labels.append(&detail);
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
            let Some(thumbnail) = row
                .first_child()
                .and_downcast::<super::super::thumbnail::ThumbnailSlot>()
            else {
                return;
            };
            let Some(labels) = thumbnail.next_sibling().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(heading) = labels.first_child().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(title) = heading.first_child().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(place) = title.next_sibling().and_downcast::<gtk::Label>() else {
                return;
            };
            let Some(detail) = heading.next_sibling().and_downcast::<gtk::Label>() else {
                return;
            };
            let fallback = if value.is_directory {
                crate::assets::icons::FOLDER
            } else {
                crate::ui::browser::icon_for_name(&value.name)
            };
            if value.is_directory {
                super::super::thumbnail::show_customized_icon(
                    &thumbnail,
                    &value.path,
                    fallback,
                    22,
                );
            } else {
                super::super::thumbnail::set_thumbnail_or_icon_for_path(
                    &thumbnail,
                    &value.path,
                    fallback,
                    22,
                    36,
                );
            }
            title.set_text(&value.name);
            detail.set_text(&result_secondary(&value));
            let position = value
                .content_hit
                .as_ref()
                .and_then(|hit| hit.locator.time_range())
                .map(|(start, _)| format_match_time(start));
            place.set_visible(position.is_some());
            if let Some(position) = position {
                place.set_text(&position);
            }
            let visible_path = display_search_path(&value.path);
            crate::ui::accessibility::set_label(&row, &visible_path);
        });
        factory.connect_unbind(|_, object| {
            super::super::thumbnail::cancel_list_item_thumbnails(object);
        });
        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.add_css_class("search-smart-list");
        list.set_single_click_activate(false);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hexpand(true)
            .vexpand(true)
            .propagate_natural_width(false)
            .propagate_natural_height(false)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let results_column = gtk::Overlay::new();
        results_column.set_width_request(352);
        results_column.set_hexpand(false);
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
        let preview_area = gtk::Box::new(gtk::Orientation::Vertical, 0);
        preview_area.add_css_class("search-smart-preview-area");
        preview_area.set_hexpand(true);
        preview_area.set_vexpand(true);
        preview_area.set_overflow(gtk::Overflow::Hidden);
        preview_area.set_layout_manager(Some(SearchSlotLayout::default()));
        preview_area.append(&preview_slot);
        let body = gtk::Paned::new(gtk::Orientation::Horizontal);
        body.add_css_class("search-smart-split");
        body.set_vexpand(true);
        body.set_hexpand(true);
        body.set_resize_start_child(false);
        body.set_resize_end_child(true);
        body.set_shrink_start_child(false);
        body.set_shrink_end_child(true);
        body.set_start_child(Some(&results_column));
        body.set_end_child(Some(&preview_area));
        body.set_position(352);
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
            self.rows
                .borrow()
                .iter()
                .position(|row| result_key(&row.path) == key)
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

fn format_match_time(start_us: u64) -> String {
    let seconds = start_us / 1_000_000;
    let hours = seconds / 3600;
    let minutes = (seconds / 60) % 60;
    let seconds = seconds % 60;
    if hours == 0 {
        format!("{minutes:02}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

fn result_secondary(item: &SearchItem) -> String {
    if let Some(hit) = &item.content_hit {
        let excerpt = hit
            .excerpt
            .split_whitespace()
            .take(24)
            .collect::<Vec<_>>()
            .join(" ");
        if !excerpt.is_empty() {
            return excerpt;
        }
    }
    item.path
        .parent()
        .map(display_search_path)
        .unwrap_or_default()
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
        if let std::collections::hash_map::Entry::Vacant(entry) = positions.entry(key) {
            entry.insert(rows.len());
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
