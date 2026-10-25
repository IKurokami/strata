// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, glib, prelude::*};

type CursorOwner = Option<(glib::WeakRef<gtk::Widget>, Option<gdk::Cursor>)>;

fn clear_cursor(owner: &RefCell<CursorOwner>) {
    if let Some((widget, cursor)) = owner.take()
        && let Some(widget) = widget.upgrade()
    {
        widget.set_cursor(cursor.as_ref());
    }
}

fn edge_at(window: &gtk::ApplicationWindow, x: f64, y: f64) -> Option<gdk::SurfaceEdge> {
    if window.is_decorated()
        || !window.is_resizable()
        || window.is_maximized()
        || window.is_fullscreen()
    {
        return None;
    }
    let width = f64::from(window.width());
    let height = f64::from(window.height());
    if width <= 0.0 || height <= 0.0 || x < 0.0 || y < 0.0 || x >= width || y >= height {
        return None;
    }
    let left = x < 6.0;
    let right = x >= width - 6.0;
    let top = y < 6.0;
    let bottom = y >= height - 6.0;
    use gdk::SurfaceEdge::*;
    match (left, right, top, bottom) {
        _ if (left || top) && x < 16.0 && y < 16.0 => Some(NorthWest),
        _ if (right || top) && x >= width - 16.0 && y < 16.0 => Some(NorthEast),
        _ if (left || bottom) && x < 16.0 && y >= height - 16.0 => Some(SouthWest),
        _ if (right || bottom) && x >= width - 16.0 && y >= height - 16.0 => Some(SouthEast),
        (_, _, true, _) => Some(North),
        (_, _, _, true) => Some(South),
        (true, _, _, _) => Some(West),
        (_, true, _, _) => Some(East),
        _ => None,
    }
}

fn cursor_name(edge: gdk::SurfaceEdge) -> &'static str {
    use gdk::SurfaceEdge::*;
    match edge {
        NorthWest => "nw-resize",
        North => "n-resize",
        NorthEast => "ne-resize",
        West => "w-resize",
        East => "e-resize",
        SouthWest => "sw-resize",
        South => "s-resize",
        SouthEast => "se-resize",
        _ => "default",
    }
}

pub(super) fn install(window: &gtk::ApplicationWindow) {
    let cursor = Rc::new(RefCell::new(None));
    let motion = gtk::EventControllerMotion::new();
    motion.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = window.downgrade();
    let hovered = cursor.clone();
    let update = Rc::new(move |x, y| {
        clear_cursor(&hovered);
        let Some(window) = weak.upgrade() else { return };
        let Some(edge) = edge_at(&window, x, y) else {
            return;
        };
        let target = window
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .unwrap_or_else(|| window.upcast());
        hovered.replace(Some((target.downgrade(), target.cursor())));
        target.set_cursor_from_name(Some(cursor_name(edge)));
    });
    let entered = update.clone();
    motion.connect_enter(move |_, x, y| entered(x, y));
    motion.connect_motion(move |_, x, y| update(x, y));
    let left = cursor.clone();
    motion.connect_leave(move |_| clear_cursor(&left));
    window.add_controller(motion);

    let press = gtk::GestureClick::builder()
        .button(gdk::BUTTON_PRIMARY)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let weak = window.downgrade();
    press.connect_pressed(move |gesture, count, x, y| {
        if count != 1 {
            return;
        }
        let Some(window) = weak.upgrade() else { return };
        let Some(edge) = edge_at(&window, x, y) else {
            return;
        };
        let Some(toplevel) = window.surface().and_downcast::<gdk::Toplevel>() else {
            return;
        };
        let Some(event) = gesture.current_event() else {
            return;
        };
        // Claim before the underlying row, divider or header can start its own drag.
        gesture.set_state(gtk::EventSequenceState::Claimed);
        toplevel.begin_resize(
            edge,
            event.device().as_ref(),
            gdk::BUTTON_PRIMARY as i32,
            x,
            y,
            event.time(),
        );
    });
    window.add_controller(press);
    for property in ["maximized", "fullscreened", "resizable", "decorated"] {
        let cursor = cursor.clone();
        window.connect_notify_local(Some(property), move |_, _| clear_cursor(&cursor));
    }
    window.connect_unmap(move |_| clear_cursor(&cursor));
}

#[cfg(test)]
mod tests;
