// SPDX-License-Identifier: MIT

use gtk::{gdk, glib, prelude::*};

pub(super) fn install(pane: &gtk::Box) {
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = pane.downgrade();
    wheel.connect_scroll(move |controller, mut dx, mut dy| {
        let modifiers = controller.current_event_state();
        if modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        ) {
            return glib::Propagation::Proceed;
        }
        let Some(pane) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let target = controller.current_event().and_then(|event| {
            let (x, y) = event.position()?;
            let native = pane.native()?;
            if event.surface() != native.surface() {
                return None;
            }
            let (ox, oy) = native.surface_transform();
            let point = native.upcast_ref::<gtk::Widget>().compute_point(
                &pane,
                &gtk::graphene::Point::new((x - ox) as f32, (y - oy) as f32),
            )?;
            pane.pick(
                f64::from(point.x()),
                f64::from(point.y()),
                gtk::PickFlags::DEFAULT,
            )
        });
        if modifiers.contains(gdk::ModifierType::SHIFT_MASK) {
            std::mem::swap(&mut dx, &mut dy);
        }
        let mut current = target;
        while let Some(widget) = current {
            // Sliders retain their own wheel behavior. Scrollbar troughs/thumbs
            // route to the surrounding document just like its content.
            if widget.is::<gtk::Range>() && !widget.is::<gtk::Scrollbar>() {
                return glib::Propagation::Proceed;
            }
            if let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>() {
                let horizontal = scroll.hscrollbar_policy() != gtk::PolicyType::Never
                    && apply_delta(&scroll.hadjustment(), dx, controller.unit());
                let vertical = scroll.vscrollbar_policy() != gtk::PolicyType::Never
                    && apply_delta(&scroll.vadjustment(), dy, controller.unit());
                if horizontal || vertical {
                    return glib::Propagation::Stop;
                }
            }
            if widget == pane {
                break;
            }
            current = widget.parent();
        }
        glib::Propagation::Proceed
    });
    pane.add_controller(wheel);
}

fn apply_delta(adjustment: &gtk::Adjustment, delta: f64, unit: gdk::ScrollUnit) -> bool {
    if !delta.is_finite() || delta == 0.0 {
        return false;
    }
    let scale = if unit == gdk::ScrollUnit::Wheel {
        adjustment.page_size().max(1.0).powf(2.0 / 3.0)
    } else {
        1.0
    };
    let upper = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    let previous = adjustment.value();
    let next = (previous + delta * scale).clamp(adjustment.lower(), upper);
    if next == previous {
        return false;
    }
    adjustment.set_value(next);
    true
}
