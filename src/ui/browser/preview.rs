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
        crate::ui::motion::cancel(&self.state.scroller, "column-scroll");
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
        let epoch = Rc::new(Cell::new(0u64));
        let changed_epoch = epoch.clone();
        let weak = self.downgrade();
        preview.connect_reveal_child_notify(move |_| {
            changed_epoch.set(changed_epoch.get().saturating_add(1));
            if let Some(view) = weak.upgrade() {
                view.release_preview_reservation();
                if view.view_mode() == BrowserMode::Columns && view.state.scroller.is_mapped() {
                    let depth = view.state.focused_depth();
                    let shell = {
                        let columns = view.state.columns.borrow();
                        depth
                            .and_then(|depth| columns.get(depth))
                            .or_else(|| columns.last())
                            .map(|column| column.shell.clone())
                    };
                    if let Some(shell) = shell {
                        view.state.reveal_column(shell);
                    }
                }
            }
        });
        let weak = self.downgrade();
        let queued = Rc::new(Cell::new(false));
        let pending = Rc::new(Cell::new(None));
        let last_page_size = Cell::new(self.state.scroller.hadjustment().page_size());
        self.state
            .scroller
            .hadjustment()
            .connect_changed(move |adjustment| {
                if last_page_size.replace(adjustment.page_size()) == adjustment.page_size() {
                    return;
                }
                let Some(view) = weak.upgrade() else { return };
                pending.set(Some((
                    epoch.get(),
                    view.state.horizontal_scroll_generation.get(),
                )));
                if queued.replace(true) {
                    return;
                }
                let weak = weak.clone();
                let queued = queued.clone();
                let pending = pending.clone();
                let epoch = epoch.clone();
                // Coalesce allocations, then use only the latest viewport. Navigation
                // or pointer scrolling after allocation owns the position instead.
                glib::idle_add_local_once(move || {
                    queued.set(false);
                    let Some((expected_epoch, expected_scroll)) = pending.take() else {
                        return;
                    };
                    let Some(view) = weak.upgrade() else { return };
                    if epoch.get() != expected_epoch
                        || view.state.horizontal_scroll_generation.get() != expected_scroll
                        || view.view_mode() != BrowserMode::Columns
                        || !view.state.scroller.is_mapped()
                    {
                        return;
                    }
                    let adjustment = view.state.scroller.hadjustment();
                    let Some(span) = view.state.focused_column_span() else {
                        return;
                    };
                    let target = span.reveal_target(
                        adjustment.value(),
                        adjustment.page_size(),
                        adjustment.lower(),
                        adjustment.upper(),
                    );
                    if (target - adjustment.value()).abs() < 0.5 {
                        return;
                    }
                    let animation_id = expected_scroll.saturating_add(1);
                    view.state.horizontal_scroll_generation.set(animation_id);
                    super::columns::animate_horizontal_scroll(
                        &view.state.scroller,
                        &adjustment,
                        target,
                        &view.state.horizontal_scroll_generation,
                        animation_id,
                    );
                });
            });
        let weak = self.downgrade();
        self.connect_view_mode_changed(move |_| {
            if let Some(view) = weak.upgrade() {
                view.release_preview_reservation();
            }
        });
    }
}
