// SPDX-License-Identifier: MIT

use super::{BrowserMode, ViewState};
use crate::{app::BrowserEvent, model::Location};
use gtk::{glib, prelude::*};
use std::{collections::HashMap, rc::Rc};

const MAX_AUTOMATIC_CHILDREN: usize = 4;

#[derive(Clone)]
struct PendingFolder {
    location: Location,
    remaining: usize,
    select: bool,
    owner_depth: usize,
    owner_cursor: Option<Location>,
}

#[derive(Default)]
pub(super) struct FolderAutofocus {
    pending: HashMap<usize, PendingFolder>,
    next: Option<PendingFolder>,
    history: bool,
}

impl ViewState {
    pub(super) fn track_folder_autofocus(self: &Rc<Self>, event: &BrowserEvent) {
        match event {
            BrowserEvent::NavigationStarting { history } => {
                self.folder_autofocus.borrow_mut().history = *history;
            }
            BrowserEvent::Reset => {
                let mut autofocus = self.folder_autofocus.borrow_mut();
                autofocus.pending.clear();
                autofocus.next = None;
            }
            BrowserEvent::ColumnsTruncated { len, .. } => {
                self.folder_autofocus
                    .borrow_mut()
                    .pending
                    .retain(|depth, _| depth < len);
            }
            BrowserEvent::ColumnAdded { depth, location } => {
                if !self.interactive || self.browser.is_chooser_mode() {
                    return;
                }
                let mut autofocus = self.folder_autofocus.borrow_mut();
                if let Some(next) = autofocus.next.take()
                    && next.location == *location
                {
                    autofocus.pending.insert(*depth, next);
                } else if !autofocus.history && self.browser.active_depth() == Some(*depth) {
                    autofocus.pending.insert(
                        *depth,
                        PendingFolder {
                            location: location.clone(),
                            remaining: MAX_AUTOMATIC_CHILDREN,
                            select: true,
                            owner_depth: *depth,
                            owner_cursor: None,
                        },
                    );
                } else if !autofocus.history
                    && self.mode.get() == BrowserMode::Columns
                    && let Some(parent) = depth.checked_sub(1)
                    && self.browser.active_depth() == Some(parent)
                    && let Some(entry) = self.browser.focused_entry()
                    && entry.location == *location
                {
                    autofocus.pending.insert(
                        *depth,
                        PendingFolder {
                            location: location.clone(),
                            remaining: 0,
                            select: false,
                            owner_depth: parent,
                            owner_cursor: Some(entry.location),
                        },
                    );
                }
            }
            BrowserEvent::LoadFailed { depth, .. } => {
                self.folder_autofocus.borrow_mut().pending.remove(depth);
            }
            BrowserEvent::LoadFinished { depth, .. } => {
                let pending = self.folder_autofocus.borrow_mut().pending.remove(depth);
                let Some(pending) = pending else {
                    return;
                };
                // Explicit reveal/transfer targets and user input always win over a load default.
                if !self.pending_select.borrow().is_empty()
                    || self.pending_location_selection.borrow().is_some()
                    || self.pending_archive_destination.borrow().is_some()
                    || self.active_rename.borrow().is_some()
                    || self.pending_new_entry.borrow().is_some()
                {
                    return;
                }
                let active = self.browser.active_depth();
                let cursor = self.browser.focused_entry().map(|entry| entry.location);
                let generation = self.browser.navigation_generation();
                let depth = *depth;
                let weak = Rc::downgrade(self);
                glib::idle_add_local_once(move || {
                    if let Some(state) = weak.upgrade()
                        && state.browser.navigation_generation() == generation
                        && state.browser.active_depth() == active
                        && state.browser.focused_entry().map(|entry| entry.location) == cursor
                        && state.browser.location_at(depth).as_ref() == Some(&pending.location)
                    {
                        state.apply_folder_autofocus(depth, pending);
                    }
                });
            }
            _ => {}
        }
    }

    fn apply_folder_autofocus(self: &Rc<Self>, depth: usize, pending: PendingFolder) {
        if self.browser.active_depth() != Some(pending.owner_depth)
            || pending.owner_cursor.as_ref().is_some_and(|location| {
                self.browser
                    .focused_entry()
                    .as_ref()
                    .map(|entry| &entry.location)
                    != Some(location)
            })
            || self.browser.visual_kind().is_some()
            || self.active_rename.borrow().is_some()
            || self.pending_new_entry.borrow().is_some()
        {
            return;
        }
        let Some(snapshot) = self.browser.column_snapshot(depth) else {
            return;
        };
        if snapshot.loading || snapshot.error.is_some() || snapshot.reveal_pending {
            return;
        }
        if !pending.select {
            let filtered = self.columns.borrow().get(depth).is_none_or(|column| {
                column.map.has_query() || column.recursive_search_active.get()
            });
            if !filtered {
                self.browser.seed_first_loaded(depth);
            }
            return;
        }
        // Restored selections and Show in folder must not open an unrelated first child.
        if !snapshot.selected_positions.is_empty() && !self.browser.selection_is_load_cursor() {
            return;
        }
        self.browser.focus_first_loaded(depth);
        if pending.owner_depth != depth
            && let Some(column) = self.columns.borrow().get(depth).cloned()
        {
            self.reveal_column(column.shell);
        }
        if self.mode.get() != BrowserMode::Columns
            || !self.columns_mirror_selection.get()
            || pending.remaining == 0
            || snapshot.truncated
            || !self.overlay.is_mapped()
            || self.browser.location_at(depth + 1).is_some()
        {
            return;
        }
        let Some((focused_depth, position, entry)) = self.browser.focused_item() else {
            return;
        };
        if focused_depth != depth
            || !entry.is_directory()
            || entry.is_symbolic_link()
            || entry.location.native_path().is_none()
        {
            return;
        }
        let single = {
            let columns = self.columns.borrow();
            let Some(column) = columns.get(depth) else {
                return;
            };
            if column.map.has_query()
                || column.recursive_search_active.get()
                || column.map.source_position(0) != Some(position)
            {
                return;
            }
            column.filtered_model.n_items() == 1
        };
        self.folder_autofocus.borrow_mut().next = Some(PendingFolder {
            location: entry.location.clone(),
            remaining: pending.remaining - 1,
            select: single,
            owner_depth: depth,
            owner_cursor: Some(entry.location.clone()),
        });
        // show_child leaves the origin active: a branching folder previews its first
        // child without moving the keyboard cursor or horizontal viewport into it.
        self.browser.show_child(depth, entry.location);
    }
}
