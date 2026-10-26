// SPDX-License-Identifier: MIT

use std::cell::Cell;

thread_local! {
    static SUPPRESSED: Cell<u32> = const { Cell::new(0) };
}

#[must_use]
pub(in crate::ui) struct SelectionPreviewGuard;

impl Drop for SelectionPreviewGuard {
    fn drop(&mut self) {
        SUPPRESSED.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

pub(in crate::ui) fn suppress_selection_preview() -> SelectionPreviewGuard {
    SUPPRESSED.with(|depth| depth.set(depth.get().saturating_add(1)));
    SelectionPreviewGuard
}

pub(in crate::ui) fn selection_preview_suppressed() -> bool {
    SUPPRESSED.with(|depth| depth.get() != 0)
}
