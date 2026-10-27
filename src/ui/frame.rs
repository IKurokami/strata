// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{glib, prelude::*};

/// Dispatches outside GTK layout, even when scheduled by a frame callback.
pub(super) struct FrameTask {
    cancelled: Rc<Cell<bool>>,
    tick: Rc<RefCell<Option<gtk::TickCallbackId>>>,
    timeout: Rc<RefCell<Option<glib::SourceId>>>,
}

impl FrameTask {
    pub(super) fn new(widget: Option<&gtk::Widget>, work: impl FnOnce() + 'static) -> Self {
        let cancelled = Rc::new(Cell::new(false));
        let tick: Rc<RefCell<Option<gtk::TickCallbackId>>> = Default::default();
        let timeout: Rc<RefCell<Option<glib::SourceId>>> = Default::default();
        let flag = cancelled.clone();
        let dispatch = move || {
            glib::idle_add_local_once(move || {
                if !flag.replace(true) {
                    work();
                }
            });
        };
        if let Some(widget) = widget.filter(|widget| widget.is_mapped()) {
            let dispatch = Rc::new(RefCell::new(Some(dispatch)));
            let for_tick = dispatch.clone();
            let tick_handle = tick.clone();
            let timeout_handle = timeout.clone();
            let id = widget.add_tick_callback(move |_, _| {
                tick_handle.take();
                if let Some(id) = timeout_handle.take() {
                    id.remove();
                }
                if let Some(dispatch) = for_tick.take() {
                    dispatch();
                }
                glib::ControlFlow::Break
            });
            tick.replace(Some(id));
            let tick_handle = tick.clone();
            let timeout_handle = timeout.clone();
            // Input and publication must still progress when a busy GPU stalls GTK's clock.
            let id = glib::timeout_add_local_once(Duration::from_millis(32), move || {
                timeout_handle.take();
                if let Some(id) = tick_handle.take() {
                    id.remove();
                }
                if let Some(dispatch) = dispatch.take() {
                    dispatch();
                }
            });
            timeout.replace(Some(id));
        } else {
            // Unmapped/headless producers have no advancing frame clock.
            dispatch();
        }
        Self {
            cancelled,
            tick,
            timeout,
        }
    }
}

impl Drop for FrameTask {
    fn drop(&mut self) {
        self.cancelled.set(true);
        if let Some(id) = self.tick.take() {
            id.remove();
        }
        if let Some(id) = self.timeout.take() {
            id.remove();
        }
    }
}

#[cfg(test)]
mod tests;
