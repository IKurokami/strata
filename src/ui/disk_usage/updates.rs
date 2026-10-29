// SPDX-License-Identifier: MIT

use crate::services::disk_usage::{Coverage, Snapshot};

pub(super) fn present(current: Option<&Snapshot>, incoming: &Snapshot) -> bool {
    let Some(current) = current else { return true };
    // Keep the last usable map interactive while recursive totals are incomplete.
    // Empty initial publications must not prevent the first useful preview.
    (!incoming.resumable && incoming.coverage != Coverage::Stale)
        || (!has_content(current) && has_content(incoming))
}

fn has_content(snapshot: &Snapshot) -> bool {
    snapshot.other_bytes > 0 || snapshot.children.iter().any(|child| child.bytes > 0)
}

pub(super) fn content_changed(current: Option<&Snapshot>, incoming: &Snapshot) -> bool {
    current.is_none_or(|current| {
        current.root != incoming.root
            || current.total != incoming.total
            || current.other_bytes != incoming.other_bytes
            || current.children.len() != incoming.children.len()
            || current
                .children
                .iter()
                .zip(&incoming.children)
                .any(|(old, new)| {
                    old.path != new.path || old.bytes != new.bytes || old.directory != new.directory
                })
    })
}

#[cfg(test)]
mod tests;
