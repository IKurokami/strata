// SPDX-License-Identifier: MIT

use super::model::{Color, PALETTE};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, Copy, Debug)]
struct Hsv {
    h: f64,
    s: f64,
    v: f64,
}

impl Hsv {
    fn from_color(Color(r, g, b): Color) -> Self {
        let [r, g, b] = [r, g, b].map(|value| f64::from(value) / 255.0);
        let maximum = r.max(g).max(b);
        let minimum = r.min(g).min(b);
        let delta = maximum - minimum;
        let hue = if delta == 0.0 {
            0.0
        } else if maximum == r {
            ((g - b) / delta).rem_euclid(6.0)
        } else if maximum == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        };
        Self {
            h: hue / 6.0,
            s: if maximum == 0.0 { 0.0 } else { delta / maximum },
            v: maximum,
        }
    }
    fn color(self) -> Color {
        let h = self.h.rem_euclid(1.0) * 6.0;
        let c = self.v * self.s;
        let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let m = self.v - c;
        let (r, g, b) = match h.floor() as u8 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let channel = |value: f64| ((value + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Color(channel(r), channel(g), channel(b))
    }
}

type Change = Rc<dyn Fn(Color)>;

pub(super) struct Picker {
    pub widget: gtk::Box,
    value: Rc<Cell<Hsv>>,
    callbacks: Rc<RefCell<Vec<Change>>>,
}

impl Picker {
    pub fn new(label: &str, initial: Color) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let value = Rc::new(Cell::new(Hsv::from_color(initial)));
        let callbacks = Rc::new(RefCell::new(Vec::<Change>::new()));
        let sv = gtk::DrawingArea::builder()
            .content_width(144)
            .content_height(100)
            .hexpand(true)
            .focusable(true)
            .build();
        sv.add_css_class("discord-color-map");
        sv.set_cursor_from_name(Some("crosshair"));
        crate::ui::accessibility::set_label(&sv, &crate::i18n::tr(label));
        let state = value.clone();
        sv.set_draw_func(move |_, cr, width, height| {
            let hsv = state.get();
            let Color(r, g, b) = Hsv {
                h: hsv.h,
                s: 1.0,
                v: 1.0,
            }
            .color();
            cr.set_source_rgb(
                f64::from(r) / 255.0,
                f64::from(g) / 255.0,
                f64::from(b) / 255.0,
            );
            let _ = cr.paint();
            let white = gtk::cairo::LinearGradient::new(0.0, 0.0, f64::from(width), 0.0);
            white.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 1.0);
            white.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
            let _ = cr.set_source(&white);
            let _ = cr.paint();
            let black = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, f64::from(height));
            black.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
            black.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 1.0);
            let _ = cr.set_source(&black);
            let _ = cr.paint();
            cr.arc(
                hsv.s * f64::from(width),
                (1.0 - hsv.v) * f64::from(height),
                4.0,
                0.0,
                std::f64::consts::TAU,
            );
            cr.set_line_width(3.0);
            cr.set_source_rgb(0.0, 0.0, 0.0);
            let _ = cr.stroke_preserve();
            cr.set_line_width(1.5);
            cr.set_source_rgb(1.0, 1.0, 1.0);
            let _ = cr.stroke();
        });
        widget.append(&sv);
        let hue = gtk::DrawingArea::builder()
            .content_width(144)
            .content_height(14)
            .hexpand(true)
            .focusable(true)
            .build();
        hue.add_css_class("discord-color-map");
        hue.set_cursor_from_name(Some("crosshair"));
        crate::ui::accessibility::set_label(&hue, &crate::i18n::tr("Hue"));
        let state = value.clone();
        hue.set_draw_func(move |_, cr, width, height| {
            let spectrum = gtk::cairo::LinearGradient::new(0.0, 0.0, f64::from(width), 0.0);
            for index in 0..=6 {
                let Color(r, g, b) = Hsv {
                    h: f64::from(index) / 6.0,
                    s: 1.0,
                    v: 1.0,
                }
                .color();
                spectrum.add_color_stop_rgb(
                    f64::from(index) / 6.0,
                    f64::from(r) / 255.0,
                    f64::from(g) / 255.0,
                    f64::from(b) / 255.0,
                );
            }
            let _ = cr.set_source(&spectrum);
            let _ = cr.paint();
            let x = state.get().h * f64::from(width);
            cr.rectangle(x - 2.0, 0.0, 4.0, f64::from(height));
            cr.set_source_rgb(1.0, 1.0, 1.0);
            let _ = cr.fill();
        });
        widget.append(&hue);
        for (area, is_hue) in [(&sv, false), (&hue, true)] {
            let gesture = gtk::GestureDrag::new();
            gesture.set_button(1);
            let origin = Rc::new(Cell::new((0.0, 0.0)));
            let update: Rc<dyn Fn(f64, f64)> = {
                let state = value.clone();
                let callbacks = callbacks.clone();
                let sv = sv.downgrade();
                let hue = hue.downgrade();
                Rc::new(move |x, y| {
                    let (Some(sv), Some(hue)) = (sv.upgrade(), hue.upgrade()) else {
                        return;
                    };
                    let mut hsv = state.get();
                    if is_hue {
                        hsv.h = (x / f64::from(hue.width().max(1))).clamp(0.0, 0.999999);
                    } else {
                        hsv.s = (x / f64::from(sv.width().max(1))).clamp(0.0, 1.0);
                        hsv.v = (1.0 - y / f64::from(sv.height().max(1))).clamp(0.0, 1.0);
                    }
                    state.set(hsv);
                    sv.queue_draw();
                    hue.queue_draw();
                    for callback in callbacks.borrow().iter() {
                        callback(hsv.color());
                    }
                })
            };
            let begin = update.clone();
            let start = origin.clone();
            gesture.connect_drag_begin(move |_, x, y| {
                start.set((x, y));
                begin(x, y);
            });
            gesture.connect_drag_update(move |_, x, y| {
                let (sx, sy) = origin.get();
                update(sx + x, sy + y);
            });
            area.add_controller(gesture);
            let keys = gtk::EventControllerKey::new();
            let state = value.clone();
            let callbacks = callbacks.clone();
            let sv = sv.downgrade();
            let hue = hue.downgrade();
            crate::ui::keys::on_press(&keys, move |_, press| {
                use crate::ui::keys::bindings::{DOWN, LEFT, RIGHT, UP};
                let mut hsv = state.get();
                let change = if press.is(LEFT) || press.is(DOWN) {
                    -0.01
                } else if press.is(RIGHT) || press.is(UP) {
                    0.01
                } else {
                    return gtk::glib::Propagation::Proceed;
                };
                if is_hue {
                    hsv.h = (hsv.h + change).rem_euclid(1.0);
                } else if press.is(LEFT) || press.is(RIGHT) {
                    hsv.s = (hsv.s + change).clamp(0.0, 1.0);
                } else {
                    hsv.v = (hsv.v + change).clamp(0.0, 1.0);
                }
                state.set(hsv);
                if let Some(sv) = sv.upgrade() {
                    sv.queue_draw();
                }
                if let Some(hue) = hue.upgrade() {
                    hue.queue_draw();
                }
                for callback in callbacks.borrow().iter() {
                    callback(hsv.color());
                }
                gtk::glib::Propagation::Stop
            });
            area.add_controller(keys);
        }
        let swatches = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        for color in PALETTE {
            let button = gtk::Button::new();
            button.add_css_class("discord-color-swatch");
            crate::ui::accessibility::set_label(&button, &color.hex());
            let mark = gtk::Label::new(None);
            mark.set_markup(&format!("<span foreground=\"{}\">●</span>", color.hex()));
            button.set_child(Some(&mark));
            let state = value.clone();
            let callbacks = callbacks.clone();
            let sv = sv.downgrade();
            let hue = hue.downgrade();
            button.connect_clicked(move |_| {
                state.set(Hsv::from_color(color));
                if let Some(sv) = sv.upgrade() {
                    sv.queue_draw();
                }
                if let Some(hue) = hue.upgrade() {
                    hue.queue_draw();
                }
                for callback in callbacks.borrow().iter() {
                    callback(color);
                }
            });
            swatches.append(&button);
        }
        widget.append(&swatches);
        Self {
            widget,
            value,
            callbacks,
        }
    }
    pub fn color(&self) -> Color {
        self.value.get().color()
    }
    pub fn connect_changed(&self, callback: impl Fn(Color) + 'static) {
        self.callbacks.borrow_mut().push(Rc::new(callback));
    }
}

#[cfg(test)]
mod tests;
