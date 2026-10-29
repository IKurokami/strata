// SPDX-License-Identifier: MIT

use gtk::prelude::*;

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
    // Hidden popovers measure as zero. Their visible contents can be measured
    // before mapping, including the menu's padding and border.
    if let Some(contents) = popover.first_child()
        && let Some(scroll) = &scroll
    {
        let width = contents.measure(gtk::Orientation::Horizontal, -1).1.max(1);
        let chrome = (contents.measure(gtk::Orientation::Vertical, width).1
            - scroll.measure(gtk::Orientation::Vertical, width).1)
            .max(0);
        let limit = (available - chrome).max(1);
        let content = scroll.child().map_or(0, |child| {
            child.measure(gtk::Orientation::Vertical, width).1
        });
        scroll.set_max_content_height(limit);
        // Prefer sliding the full menu over shrinking it to one side of the
        // pointer. Oversized menus still scroll within the monitor's work area.
        scroll.set_min_content_height(content.min(limit));
    }
    // GTK's horizontal placement permits vertical sliding; Bottom only flips
    // vertically and can reject a menu that fits neither half of the screen.
    popover.set_position(gtk::PositionType::Right);
    popover.set_valign(gtk::Align::Start);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        point.x().round() as i32,
        point.y().round() as i32,
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
fn vertical_bounds(
    anchor: &gtk::Widget,
    _parent: &gtk::Widget,
    _point: gtk::graphene::Point,
) -> Option<(i32, i32)> {
    let surface = anchor.native()?.surface()?;
    let monitor = anchor.display().monitor_at_surface(&surface)?;
    Some((0, monitor.geometry().height()))
}
