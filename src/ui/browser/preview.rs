// SPDX-License-Identifier: MIT

use super::columns::ColumnSpan;
use super::*;

impl ViewState {
    pub(super) fn column_span(&self, depth: usize) -> Option<ColumnSpan> {
        let columns = self.columns.borrow();
        let column = columns.get(depth)?;
        let left = columns[..depth]
            .iter()
            .map(column_width)
            .fold(0, i32::saturating_add);
        Some(ColumnSpan {
            left: f64::from(left),
            right: f64::from(left.saturating_add(column_width(column))),
            trailing: f64::from(self.columns_width(depth.saturating_add(1)..)),
        })
    }

    /// The focused column, or the one a pending click is opening, so the reserved
    /// slot does not resize between the press and the open.
    fn focused_depth(&self) -> Option<usize> {
        self.pointer_opening_depth()
            .map(|parent| parent.saturating_add(1))
            .or_else(|| self.browser.active_depth())
    }

    fn focused_column_span(&self) -> Option<ColumnSpan> {
        let count = self.columns.borrow().len();
        let depth = self
            .focused_depth()
            .filter(|depth| *depth < count)
            .or_else(|| count.checked_sub(1))?;
        self.column_span(depth)
    }

    fn navigated_len(&self) -> usize {
        let count = self.columns.borrow().len();
        // Columns beyond a folder a click is opening are about to close, so they
        // borrow no preview space in the meantime.
        if self.pointer_opening_depth().is_some() {
            return count;
        }
        self.browser
            .active_depth()
            .map_or(count, |depth| depth.saturating_add(1).min(count))
    }

    fn columns_width(
        &self,
        range: impl std::slice::SliceIndex<[ColumnView], Output = [ColumnView]>,
    ) -> i32 {
        self.columns.borrow().get(range).map_or(0, |columns| {
            columns
                .iter()
                .map(column_width)
                .fold(0, i32::saturating_add)
        })
    }
}

fn column_width(column: &ColumnView) -> i32 {
    column
        .shell
        .width()
        .max(column.shell.width_request())
        .max(COLUMN_WIDTH)
}

impl BrowserView {
    pub(in crate::ui) fn is_resizing_columns(&self) -> bool {
        self.state.column_resizing.get()
    }

    /// Bounds Miller column widths. The first limit, and the first after a text-size
    /// change, also narrow wider open columns; a shrinking window does not.
    pub(in crate::ui) fn set_column_width_limit(&self, limit: i32) {
        self.state.column_width_limit.set(Some(limit));
        if !self.state.clamp_to_next_column_limit.replace(false) {
            return;
        }
        let limit = limit.max(super::columns::column_floor());
        for column in self.state.columns.borrow().iter() {
            if column.shell.width_request() > limit {
                // Stops a running width animation from restoring the oversized width.
                column
                    .animation_generation
                    .set(column.animation_generation.get().saturating_add(1));
                column.shell.set_size_request(limit, -1);
            }
        }
    }

    pub(in crate::ui) fn preview_navigated_width(&self, available: i32) -> i32 {
        if self.view_mode() != BrowserMode::Columns {
            return single_pane_preview_reservation(available);
        }
        self.state.columns_width(..self.state.navigated_len())
    }

    pub(in crate::ui) fn preview_trailing_width(&self) -> i32 {
        if self.view_mode() != BrowserMode::Columns {
            return 0;
        }
        self.state.columns_width(self.state.navigated_len()..)
    }

    pub(in crate::ui) fn preview_navigation_width(&self) -> i32 {
        self.state
            .focused_column_span()
            .map_or(COLUMN_WIDTH, |span| span.width() as i32)
    }

    pub(in crate::ui) fn preview_standard_navigation_width(&self) -> i32 {
        self.state
            .focused_column_span()
            .map_or(COLUMN_WIDTH, |span| {
                span.width().min(f64::from(COLUMN_WIDTH)) as i32
            })
    }

    pub(in crate::ui) fn release_preview_reservation(&self) {
        self.state.columns_widget.set_margin_end(0);
        self.state.horizontal_scroll_generation.set(
            self.state
                .horizontal_scroll_generation
                .get()
                .saturating_add(1),
        );
    }

    pub(in crate::ui) fn clear_preview_scroll_space(&self) {
        self.state.columns_widget.set_margin_end(0);
    }

    pub(in crate::ui) fn bind_preview_scrolling(&self, preview: &gtk::Revealer) {
        let weak = self.downgrade();
        let weak_preview = preview.downgrade();
        let generation = Rc::new(Cell::new(0u64));
        let changed_generation = generation.clone();
        preview.connect_reveal_child_notify(move |_| {
            changed_generation.set(changed_generation.get().saturating_add(1));
        });
        let last_page_size = Cell::new(self.state.scroller.hadjustment().page_size());
        self.state
            .scroller
            .hadjustment()
            .connect_changed(move |adjustment| {
                let page_changed =
                    last_page_size.replace(adjustment.page_size()) != adjustment.page_size();
                let generation_id = generation.get();
                let generation = generation.clone();
                let weak = weak.clone();
                let weak_preview = weak_preview.clone();
                // GtkViewport must finish allocating before its scroll value is changed.
                glib::idle_add_local_once(move || {
                    let Some(view) = weak.upgrade() else { return };
                    if generation.get() != generation_id || view.view_mode() != BrowserMode::Columns
                    {
                        return;
                    }
                    let is_open = weak_preview
                        .upgrade()
                        .is_some_and(|preview| preview.reveals_child());
                    let adjustment = view.state.scroller.hadjustment();
                    if is_open {
                        view.state.horizontal_scroll_generation.set(
                            view.state
                                .horizontal_scroll_generation
                                .get()
                                .saturating_add(1),
                        );
                        if let Some(span) = view.state.focused_column_span() {
                            let target = span.reveal_target(
                                adjustment.value(),
                                adjustment.page_size(),
                                adjustment.lower(),
                                adjustment.upper(),
                            );
                            let fully_visible = span.left >= adjustment.value()
                                && span.right <= adjustment.value() + adjustment.page_size();
                            if view.state.columns_widget.margin_end() == 0
                                || !fully_visible
                                || target >= adjustment.value()
                            {
                                adjustment.set_value(target);
                            }
                        }
                    } else if page_changed {
                        view.clear_preview_scroll_space();
                        if let Some(span) = view.state.focused_column_span() {
                            let target = span.reveal_target(
                                adjustment.value(),
                                adjustment.page_size(),
                                adjustment.lower(),
                                adjustment.upper(),
                            );
                            if target != adjustment.value() {
                                adjustment.set_value(target);
                            }
                        }
                    }
                });
            });
        let weak = self.downgrade();
        self.connect_view_mode_changed(move |_| {
            if let Some(view) = weak.upgrade() {
                view.clear_preview_scroll_space();
            }
        });
    }
}
