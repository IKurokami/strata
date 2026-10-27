// SPDX-License-Identifier: MIT

use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

pub(super) const CONTENT: Duration = Duration::from_millis(120);
pub(super) const PANEL: Duration = Duration::from_millis(180);
pub(super) const MODAL: Duration = Duration::from_millis(180);
pub(super) const FEEDBACK: Duration = Duration::from_millis(180);

type ActiveMotion = (glib::WeakRef<gtk::Widget>, &'static str, Rc<Cell<bool>>);

thread_local! {
    static REDUCE_MOTION: Cell<bool> = const { Cell::new(false) };
    static ACTIVE: RefCell<Vec<ActiveMotion>> = const { RefCell::new(Vec::new()) };
    static REDUCED_CSS: RefCell<Option<(gtk::gdk::Display, gtk::CssProvider)>> = const { RefCell::new(None) };
}

pub(super) fn cancel(widget: &impl IsA<gtk::Widget>, channel: &'static str) {
    ACTIVE.with_borrow_mut(|active| {
        active.retain(|(owner, key, running)| {
            let Some(owner) = owner.upgrade() else {
                return false;
            };
            if owner == *widget.as_ref() && *key == channel {
                running.set(false);
            }
            running.get()
        });
    });
}

pub(super) fn animate(
    widget: &impl IsA<gtk::Widget>,
    channel: Option<&'static str>,
    duration: Duration,
    frame: impl Fn(&gtk::Widget, Duration) + 'static,
    on_done: impl FnOnce() + 'static,
) {
    let widget = widget.as_ref();
    if let Some(channel) = channel {
        cancel(widget, channel);
    }
    let running = Rc::new(Cell::new(true));
    if let Some(channel) = channel {
        ACTIVE
            .with_borrow_mut(|active| active.push((widget.downgrade(), channel, running.clone())));
    }
    if !animations_enabled() || duration.is_zero() {
        frame(widget, duration);
        // Leave input dispatch before teardown, without waiting for the UI to
        // become idle: indexing or rendering must not starve modal completion.
        glib::timeout_add_local_once(Duration::ZERO, move || {
            if running.replace(false) {
                on_done();
            }
        });
        return;
    }
    let done = Rc::new(RefCell::new(Some(on_done)));
    let tick_id = Rc::new(RefCell::new(None));
    let frame = Rc::new(frame);
    let tick_frame = frame.clone();
    let tick_running = running.clone();
    let tick_done = done.clone();
    let tick_handle = tick_id.clone();
    let started = Instant::now();
    frame(widget, Duration::ZERO);
    if !running.get() {
        return;
    }
    let id = widget.add_tick_callback(move |widget, _| {
        if !tick_running.get() {
            tick_handle.take();
            return glib::ControlFlow::Break;
        }
        let elapsed = if animations_enabled() {
            started.elapsed().min(duration)
        } else {
            duration
        };
        tick_frame(widget, elapsed);
        if elapsed < duration && tick_running.get() {
            return glib::ControlFlow::Continue;
        }
        tick_handle.take();
        if tick_running.replace(false)
            && let Some(done) = tick_done.take()
        {
            done();
        }
        glib::ControlFlow::Break
    });
    tick_id.replace(Some(id));
    let weak = widget.downgrade();
    // A saturated GPU may stop frame callbacks while the main loop still accepts
    // input. Completion and cancellation cannot depend on another rendered frame.
    glib::timeout_add_local_once(duration, move || {
        if let Some(id) = tick_id.take() {
            id.remove();
        }
        if running.replace(false) {
            if let Some(widget) = weak.upgrade() {
                frame(&widget, duration);
            }
            if let Some(done) = done.take() {
                done();
            }
        } else {
            done.take();
        }
    });
}

pub(super) fn class_once(widget: &impl IsA<gtk::Widget>, class: &'static str, duration: Duration) {
    let widget = widget.as_ref();
    if !animations_enabled() {
        cancel(widget, class);
        widget.remove_css_class(class);
        return;
    }
    if widget.has_css_class(class) {
        return;
    }
    cancel(widget, class);
    let running = Rc::new(Cell::new(true));
    ACTIVE.with_borrow_mut(|active| {
        active.push((widget.downgrade(), class, running.clone()));
    });
    widget.add_css_class(class);
    let weak = widget.downgrade();
    // CSS already owns the frame clock. Class cleanup needs only one deadline.
    glib::timeout_add_local_once(duration, move || {
        if running.replace(false)
            && let Some(widget) = weak.upgrade()
        {
            widget.remove_css_class(class);
        }
    });
}

pub(super) fn smooth(progress: f64) -> f64 {
    let remaining = 1.0 - progress.clamp(0.0, 1.0);
    1.0 - remaining * remaining * remaining
}

pub(super) fn set_reduce_motion(reduced: bool) {
    REDUCE_MOTION.set(reduced);
    sync_reduced_css();
}

fn sync_reduced_css() {
    REDUCED_CSS.with_borrow_mut(|installed| {
        if !REDUCE_MOTION.get() {
            if let Some((display, provider)) = installed.take() {
                gtk::style_context_remove_provider_for_display(&display, &provider);
            }
        } else if installed.is_none()
            && let Some(display) = gtk::gdk::Display::default()
        {
            let provider = gtk::CssProvider::new();
            provider.load_from_string("* { transition: none; animation: none; }");
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
            *installed = Some((display, provider));
        }
    });
}

pub(super) fn bind_stack(stack: &gtk::Stack, preferences: &super::preferences::PreferenceManager) {
    preferences.bind_preference(
        stack,
        |preferences| preferences.reduce_motion(),
        |widget, reduced| {
            if let Some(stack) = widget.downcast_ref::<gtk::Stack>() {
                stack.set_transition_duration(if reduced {
                    0
                } else {
                    CONTENT.as_millis() as u32
                });
            }
        },
    );
}

pub(super) fn animations_enabled() -> bool {
    sync_reduced_css();
    !REDUCE_MOTION.get()
        && gtk::Settings::default()
            .map(|settings| settings.is_gtk_enable_animations())
            .unwrap_or(true)
}

pub(super) fn emphasized_deceleration(progress: f64) -> f64 {
    cubic_bezier(progress, (0.16, 1.0), (0.3, 1.0))
}

/// The mirror of [`emphasized_deceleration`] for things leaving the screen:
/// slow to start, fast to finish, so an exit doesn't front-load its motion.
pub(super) fn emphasized_acceleration(progress: f64) -> f64 {
    cubic_bezier(progress, (0.3, 0.0), (0.8, 0.15))
}

fn cubic_bezier(progress: f64, first: (f64, f64), second: (f64, f64)) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    if progress == 0.0 || progress == 1.0 {
        return progress;
    }
    let mut lower = 0.0;
    let mut upper = 1.0;
    for _ in 0..16 {
        let time = (lower + upper) / 2.0;
        if cubic_coordinate(time, first.0, second.0) < progress {
            lower = time;
        } else {
            upper = time;
        }
    }
    cubic_coordinate((lower + upper) / 2.0, first.1, second.1)
}

fn cubic_coordinate(time: f64, first: f64, second: f64) -> f64 {
    let inverse = 1.0 - time;
    3.0 * inverse * inverse * time * first
        + 3.0 * inverse * time * time * second
        + time * time * time
}

#[cfg(test)]
mod tests;
