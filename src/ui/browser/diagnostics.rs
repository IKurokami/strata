// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use gtk::{gdk, glib, prelude::*};

static NEXT_NAVIGATION: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub(super) struct NavigationTrace {
    current: Option<(u64, Instant)>,
    publication_seen: bool,
    frame: Option<(gdk::FrameClock, Rc<RefCell<Option<glib::SignalHandlerId>>>)>,
}

impl NavigationTrace {
    pub(super) fn begin(&mut self) {
        self.disconnect();
        let id = NEXT_NAVIGATION.fetch_add(1, Ordering::Relaxed);
        self.current = Some((id, Instant::now()));
        self.publication_seen = false;
        tracing::info!(navigation_id = id, "UI navigation dispatch started");
    }

    pub(super) fn published(&mut self, widget: &gtk::Widget, entries: usize, started: Instant) {
        let elapsed_us = started.elapsed().as_micros() as u64;
        let navigation_id = self.current.map_or(0, |(id, _)| id);
        if elapsed_us >= 8_000 {
            tracing::warn!(
                navigation_id,
                entries,
                elapsed_us,
                "UI publication exceeded frame budget"
            );
        } else {
            tracing::debug!(
                navigation_id,
                entries,
                elapsed_us,
                "UI publication dispatch completed"
            );
        }
        if self.publication_seen {
            return;
        }
        self.publication_seen = true;
        let Some((id, requested)) = self.current else {
            return;
        };
        tracing::info!(
            navigation_id = id,
            entries,
            dispatch_us = elapsed_us,
            elapsed_ms = requested.elapsed().as_millis() as u64,
            "UI first model publication completed"
        );
        self.disconnect();
        let Some(clock) = widget.frame_clock().filter(|_| widget.is_mapped()) else {
            tracing::debug!(navigation_id = id, "UI publication awaiting widget mapping");
            return;
        };
        let handler_id = Rc::new(RefCell::new(None));
        let for_paint = handler_id.clone();
        let published = Instant::now();
        let handler = clock.connect_after_paint(move |clock| {
            if let Some(handler) = for_paint.take() {
                clock.disconnect(handler);
            }
            // GTK after-paint is a CPU frame-clock checkpoint, not a physical
            // display/presentation timestamp or proof that every row is visible.
            tracing::info!(
                navigation_id = id,
                elapsed_ms = requested.elapsed().as_millis() as u64,
                publication_to_after_paint_us = published.elapsed().as_micros() as u64,
                "UI first frame after model publication"
            );
        });
        handler_id.replace(Some(handler));
        self.frame = Some((clock, handler_id));
        widget.queue_draw();
    }

    fn disconnect(&mut self) {
        if let Some((clock, handler)) = self.frame.take()
            && let Some(handler) = handler.take()
        {
            clock.disconnect(handler);
        }
    }
}

impl Drop for NavigationTrace {
    fn drop(&mut self) {
        self.disconnect();
    }
}
