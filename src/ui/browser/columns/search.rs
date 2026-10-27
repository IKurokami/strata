// SPDX-License-Identifier: MIT

use super::bitset_positions;
use crate::services::SearchItem;
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
};

#[cfg(test)]
mod tests;

pub(super) fn update_results(
    model: &gtk::StringList,
    results: &RefCell<Vec<SearchItem>>,
    selection: &gtk::MultiSelection,
    syncing: &Cell<bool>,
    items: Vec<SearchItem>,
) -> bool {
    let selected_paths: HashSet<_> = bitset_positions(&selection.selection())
        .into_iter()
        .filter_map(|position| {
            results
                .borrow()
                .get(position as usize)
                .map(|item| item.path.clone())
        })
        .collect();
    let was_syncing = syncing.replace(true);
    let same = |old: &SearchItem, new: &SearchItem| {
        old.path == new.path && old.name == new.name && old.is_directory == new.is_directory
    };
    let (prefix, removed, added) = {
        let old = results.borrow();
        let prefix = old
            .iter()
            .zip(&items)
            .take_while(|(old, new)| same(old, new))
            .count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(items[prefix..].iter().rev())
            .take_while(|(old, new)| same(old, new))
            .count();
        (
            prefix,
            old.len() - prefix - suffix,
            items.len() - prefix - suffix,
        )
    };
    // Row binding reads the authoritative results synchronously during notification.
    results.replace(items.clone());
    if removed > 0 || added > 0 {
        let names: Vec<_> = items[prefix..prefix + added]
            .iter()
            .map(|item| item.name.as_str())
            .collect();
        model.splice(prefix as u32, removed as u32, &names);
    }
    let selected = gtk::Bitset::new_empty();
    for (position, item) in items.iter().enumerate() {
        if selected_paths.contains(&item.path) {
            selected.add(position as u32);
        }
    }
    selection.set_selection(&selected, &gtk::Bitset::new_range(0, items.len() as u32));
    syncing.set(was_syncing);
    let retained: HashSet<_> = items
        .iter()
        .filter(|item| selected_paths.contains(&item.path))
        .map(|item| item.path.clone())
        .collect();
    retained != selected_paths
}
