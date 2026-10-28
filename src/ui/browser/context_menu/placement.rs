// SPDX-License-Identifier: MIT

use gtk::prelude::*;

#[cfg(windows)]
pub(super) fn position(popover: &gtk::Popover, anchor: &gtk::Widget, x: f64, y: f64) {
    let Some(parent) = popover.parent() else {
        return;
    };
    let Some(point) = anchor.compute_point(&parent, &gtk::graphene::Point::new(x as f32, y as f32))
    else {
        return;
    };
    let margin = super::CONTEXT_MENU_EDGE_MARGIN;
    let (top, bottom) =
        vertical_bounds(anchor, &parent, point).unwrap_or((0, parent.height().max(1)));
    let available = bottom.saturating_sub(top).saturating_sub(2 * margin).max(1);
    let scroll = first_scroll(popover.upcast_ref());
    if let Some(scroll) = &scroll {
        scroll.set_min_content_height(0);
        scroll.set_max_content_height(available);
        scroll.set_propagate_natural_height(true);
    }
    let width = popover.measure(gtk::Orientation::Horizontal, -1).1.max(1);
    if let Some(scroll) = &scroll {
        let chrome = (popover.measure(gtk::Orientation::Vertical, width).1
            - scroll.measure(gtk::Orientation::Vertical, width).1)
            .max(0);
        let limit = (available - chrome).max(1);
        let content = scroll.child().map_or(0, |child| {
            child.measure(gtk::Orientation::Vertical, width).1
        });
        scroll.set_max_content_height(limit);
        // A zero minimum lets GTK shrink the menu to the gap below the pointer
        // before trying another position. Request the usable height before map.
        scroll.set_min_content_height(content.min(limit));
    }
    let height = popover
        .measure(gtk::Orientation::Vertical, width)
        .1
        .min(available)
        .max(1);
    let menu_y = (point.y().round() as i32)
        .clamp(top + margin, (bottom - margin - height).max(top + margin));
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        point.x().round() as i32,
        menu_y,
        1,
        1,
    )));
}

fn first_scroll(widget: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
    if let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>() {
        return Some(scroll.clone());
    }
    let mut next = widget.first_child();
    while let Some(child) = next {
        if child.is_visible()
            && child.is_child_visible()
            && let Some(scroll) = first_scroll(&child)
        {
            return Some(scroll);
        }
        next = child.next_sibling();
    }
    None
}

#[cfg(windows)]
fn vertical_bounds(
    anchor: &gtk::Widget,
    parent: &gtk::Widget,
    point: gtk::graphene::Point,
) -> Option<(i32, i32)> {
    let window = anchor.root()?.downcast::<gtk::ApplicationWindow>().ok()?;
    let handle = crate::ui::windows_window_state::handle(&window)?;
    let surface = window.surface()?;
    let scale = surface.scale();
    let at = parent.compute_point(&window, &point)?;
    let [top, bottom] = strata_win_shell::window::popup_vertical_bounds(
        handle,
        [
            (f64::from(at.x()) * scale).round() as i32,
            (f64::from(at.y()) * scale).round() as i32,
        ],
    )
    .ok()?;
    let origin = window.compute_point(parent, &gtk::graphene::Point::new(0.0, 0.0))?;
    Some((
        (f64::from(top) / scale + f64::from(origin.y())).ceil() as i32,
        (f64::from(bottom) / scale + f64::from(origin.y())).floor() as i32,
    ))
}

#[cfg(not(windows))]
pub(super) fn position(popover: &gtk::Popover, anchor: &gtk::Widget, x: f64, y: f64) {
    let Some(parent) = popover.parent() else {
        return;
    };
    let Some(point) = anchor.compute_point(&parent, &gtk::graphene::Point::new(x as f32, y as f32))
    else {
        return;
    };
    if let Some(scroll) = first_scroll(popover.upcast_ref()) {
        let height = anchor
            .native()
            .and_then(|native| native.surface())
            .and_then(|surface| anchor.display().monitor_at_surface(&surface))
            .map_or(600, |monitor| monitor.geometry().height());
        scroll.set_max_content_height((height - 2 * super::CONTEXT_MENU_EDGE_MARGIN).max(1));
        scroll.set_propagate_natural_height(true);
    }
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        point.x().round() as i32,
        point.y().round() as i32,
        1,
        1,
    )));
}
