// SPDX-License-Identifier: MIT

use crate::{
    app::Browser,
    model::{FileEntry, Location},
    services::disk_usage::{self, Coverage, Snapshot},
};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
};

mod accessibility;
mod layout;
#[cfg(test)]
mod tests;

pub(super) struct DiskUsageView {
    pub widget: gtk::Box,
    canvas: accessibility::Canvas,
    status: gtk::Label,
    stage: gtk::Overlay,
    context_host: RefCell<Option<gtk::Box>>,
    active_edits: super::collection_edit::ActiveEdits,
    browser: Weak<Browser>,
    snapshot: RefCell<Option<Snapshot>>,
    tiles: RefCell<Vec<(PathBuf, layout::Rect)>>,
    hovered: RefCell<Option<PathBuf>>,
    other_rect: Cell<Option<layout::Rect>>,
    root: RefCell<Option<PathBuf>>,
    epoch: Cell<u64>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
    previous_tiles: RefCell<std::collections::HashMap<PathBuf, layout::Rect>>,
    transition: Cell<Option<std::time::Instant>>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    context: RefCell<Option<Weak<super::browser::ViewState>>>,
    context_trigger: RefCell<Option<Rc<dyn Fn(f64, f64)>>>,
}

impl DiskUsageView {
    pub fn new(
        browser: &Rc<Browser>,
        active_edits: super::collection_edit::ActiveEdits,
    ) -> Rc<Self> {
        let canvas = accessibility::Canvas::new();
        super::accessibility::set_label(&canvas, &crate::i18n::tr("Disk Usage"));
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.add_css_class("dim-label");
        status.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let stage = gtk::Overlay::new();
        stage.set_child(Some(&canvas));
        widget.append(&stage);
        widget.append(&status);
        let view = Rc::new(Self {
            widget,
            canvas,
            status,
            stage,
            active_edits,
            context_host: RefCell::new(None),
            browser: Rc::downgrade(browser),
            snapshot: RefCell::new(None),
            tiles: RefCell::new(Vec::new()),
            other_rect: Cell::new(None),
            hovered: RefCell::new(None),
            root: RefCell::new(None),
            epoch: Cell::new(0),
            task: RefCell::new(None),
            previous_tiles: RefCell::new(std::collections::HashMap::new()),
            transition: Cell::new(None),
            tick: RefCell::new(None),
            context: RefCell::new(None),
            context_trigger: RefCell::new(None),
        });
        let weak = Rc::downgrade(&view);
        view.canvas.connect_map(move |_| {
            if let Some(view) = weak.upgrade()
                && let Some(root) = view.root.borrow().clone()
            {
                disk_usage::promote(root);
            }
        });
        let weak = Rc::downgrade(&view);
        view.canvas.set_draw_func(move |canvas, cr, width, height| {
            if let Some(view) = weak.upgrade() {
                view.draw(canvas, cr, width, height);
            }
        });
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&view);
        motion.connect_motion(move |_, x, y| {
            if let Some(view) = weak.upgrade() {
                let hit = view.hit(x, y);
                if *view.hovered.borrow() != hit {
                    view.hovered.replace(hit);
                    view.update_status();
                    view.canvas.queue_draw();
                }
            }
        });
        let weak = Rc::downgrade(&view);
        motion.connect_leave(move |_| {
            if let Some(view) = weak.upgrade() {
                view.hovered.take();
                view.update_status();
                view.canvas.queue_draw();
            }
        });
        view.canvas.add_controller(motion);
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let weak = Rc::downgrade(&view);
        click.connect_pressed(move |gesture, count, x, y| {
            let Some(view) = weak.upgrade() else {
                return;
            };
            view.hovered.replace(view.hit(x, y));
            if gesture.current_button() == 1 {
                view.canvas.grab_focus();
                if view
                    .other_rect
                    .get()
                    .is_some_and(|rect| rect.contains(x, y))
                {
                    if let Some(state) = view.context.borrow().as_ref().and_then(Weak::upgrade) {
                        super::browser::BrowserView::from_state(state)
                            .set_view_mode(super::browser_modes::BrowserMode::List);
                    }
                    return;
                }
                if let Some((browser, depth, position, _)) = view.target() {
                    browser.select(depth, position);
                    if count == 2 {
                        browser.activate_in_place(depth, position);
                    }
                }
            } else if gesture.current_button() == 3
                && let Some(trigger) = view.context_trigger.borrow().clone()
            {
                trigger(x, y);
            }
        });
        view.canvas.add_controller(click);
        let key = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&view);
        super::keys::on_press(&key, move |_, press| {
            use super::keys::bindings::{
                CONTEXT_MENU, CONTEXT_MENU_F10, DOWN, ENTER, LEFT, RIGHT, UP,
            };
            let Some(view) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if press.is(CONTEXT_MENU) || press.is(CONTEXT_MENU_F10) {
                let selected = view
                    .browser
                    .upgrade()
                    .and_then(|browser| browser.focused_entry())
                    .and_then(|entry| entry.location.native_path().map(Path::to_path_buf));
                view.hovered.replace(selected.clone());
                if let Some(rect) = selected.and_then(|path| {
                    view.tiles
                        .borrow()
                        .iter()
                        .find(|(item, _)| *item == path)
                        .map(|(_, rect)| *rect)
                }) && let Some(trigger) = view.context_trigger.borrow().clone()
                {
                    trigger(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
                }
                return glib::Propagation::Stop;
            }
            if press.is(ENTER) {
                if let Some(browser) = view.browser.upgrade() {
                    browser.activate_focused_in_place();
                }
                return glib::Propagation::Stop;
            }
            let direction = if press.is(LEFT) {
                (-1.0, 0.0)
            } else if press.is(RIGHT) {
                (1.0, 0.0)
            } else if press.is(UP) {
                (0.0, -1.0)
            } else if press.is(DOWN) {
                (0.0, 1.0)
            } else {
                return glib::Propagation::Proceed;
            };
            view.move_selection(direction);
            glib::Propagation::Stop
        });
        view.canvas.add_controller(key);
        let drag = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
            .build();
        let weak = Rc::downgrade(&view);
        drag.connect_prepare(move |_, x, y| {
            let view = weak.upgrade()?;
            view.hovered.replace(view.hit(x, y));
            let (_, _, _, entry) = view.target()?;
            super::browser::file_drag_content(&[entry])
        });
        super::file_drag::install(&drag, "disk_usage");
        view.canvas.add_controller(drag);
        view
    }

    pub fn invalidate(&self) {
        if let Some(root) = self.root.borrow().clone() {
            disk_usage::invalidate(root);
        }
    }

    pub fn set_context(&self, state: Weak<super::browser::ViewState>) {
        self.context.replace(Some(state));
    }

    pub fn refresh(self: &Rc<Self>) {
        let Some(browser) = self.browser.upgrade() else {
            return;
        };
        let location = browser.active_location();
        let root = location
            .as_ref()
            .and_then(Location::native_path)
            .map(Path::to_path_buf);
        if *self.root.borrow() == root {
            self.canvas.queue_draw();
            return;
        }
        self.stop();
        self.root.replace(root.clone());
        self.snapshot.take();
        self.tiles.borrow_mut().clear();
        if let Some(location) = location.as_ref() {
            super::accessibility::describe_pane(
                &self.widget,
                &location.display_name(),
                super::browser_modes::BrowserMode::DiskUsage,
            );
        }
        self.canvas.queue_draw();
        self.update_status();
        let Some(root) = root else {
            self.status.set_text(&crate::i18n::tr("Unavailable"));
            return;
        };
        // Network paths require a separate explicit scan policy.
        if crate::adapters::with_mount_snapshot(|snapshot| snapshot.table.is_network_path(&root)) {
            self.status.set_text(&crate::i18n::tr("Unavailable"));
            return;
        }
        if let (Some(state), Some(depth)) = (
            self.context.borrow().as_ref().and_then(Weak::upgrade),
            browser.active_depth(),
        ) {
            let weak = Rc::downgrade(self);
            let resolve = Rc::new(move |_: &gtk::Widget| {
                weak.upgrade()?
                    .target()
                    .map(|(_, _, position, entry)| (Some(position), entry))
            });
            let anchor = gtk::Box::new(gtk::Orientation::Vertical, 0);
            anchor.set_hexpand(true);
            anchor.set_vexpand(true);
            anchor.set_can_target(false);
            self.stage.add_overlay(&anchor);
            self.context_host.replace(Some(anchor.clone()));
            self.context_trigger
                .replace(Some(super::browser::install_resolved_item_context_menu(
                    &state,
                    &anchor.clone().upcast(),
                    resolve,
                    depth,
                )));
        }
        let receiver = disk_usage::subscribe(root.clone());
        let epoch = self.epoch.get();
        let weak = Rc::downgrade(self);
        self.task.replace(Some(glib::spawn_future_local(async move {
            while let Ok(snapshot) = receiver.recv().await {
                let Some(view) = weak.upgrade() else {
                    break;
                };
                if view.epoch.get() != epoch || view.root.borrow().as_ref() != Some(&snapshot.root)
                {
                    break;
                }
                let labels: Vec<_> = snapshot
                    .children
                    .iter()
                    .filter(|child| child.bytes > 0)
                    .map(|child| {
                        format!(
                            "{} · {}",
                            child.path.file_name().unwrap_or_default().to_string_lossy(),
                            super::browser::format_file_size(child.bytes)
                        )
                    })
                    .collect();
                view.canvas.set_items(&labels);
                view.animate_update();
                view.snapshot.replace(Some(snapshot));
                view.update_status();
                view.canvas.queue_draw();
            }
        })));
    }

    fn animate_update(self: &Rc<Self>) {
        self.transition.set(None);
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
        if super::preferences::PreferenceManager::shared().reduce_motion()
            || self.tiles.borrow().is_empty()
        {
            return;
        }
        self.previous_tiles
            .replace(self.tiles.borrow().iter().cloned().collect());
        self.transition.set(Some(std::time::Instant::now()));
        let weak = Rc::downgrade(self);
        self.tick
            .replace(Some(self.canvas.add_tick_callback(move |_, _| {
                let Some(view) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                view.canvas.queue_draw();
                if super::preferences::PreferenceManager::shared().reduce_motion()
                    || view.transition.get().is_none_or(|started| {
                        started.elapsed() >= std::time::Duration::from_millis(140)
                    })
                {
                    view.transition.set(None);
                    view.tick.borrow_mut().take();
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            })));
    }

    pub fn stop(&self) {
        super::collection_edit::cancel(&self.active_edits);
        self.transition.set(None);
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
        self.previous_tiles.borrow_mut().clear();
        self.epoch.set(self.epoch.get().wrapping_add(1));
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
        self.root.take();
        self.context_trigger.take();
        if let Some(anchor) = self.context_host.borrow_mut().take() {
            self.stage.remove_overlay(&anchor);
        }
        self.canvas.set_items(&[]);
    }

    pub fn rename_target(&self, entry: &FileEntry) -> Option<super::collection_edit::EditTarget> {
        let path = entry.location.native_path()?;
        let rect = self
            .tiles
            .borrow()
            .iter()
            .find(|(item, _)| item == path)
            .map(|(_, rect)| *rect)?;
        let field = gtk::Entry::new();
        field.add_css_class("inline-rename");
        field.set_halign(gtk::Align::Start);
        field.set_valign(gtk::Align::Start);
        field.set_margin_start(rect.x.max(0.0) as i32 + 6);
        field.set_margin_top(rect.y.max(0.0) as i32 + 4);
        field.set_width_chars(20);
        field.set_max_width_chars(40);
        self.stage.add_overlay(&field);
        let display = gtk::Label::new(None);
        let widgets = super::collection_edit::EditWidgets::new(&field, &display);
        widgets.bind(&entry.location);
        let mut target = super::collection_edit::EditTarget::from(widgets);
        let stage = self.stage.downgrade();
        let weak_field = field.downgrade();
        target.finish = Some(Rc::new(move || {
            if let (Some(stage), Some(field)) = (stage.upgrade(), weak_field.upgrade()) {
                stage.remove_overlay(&field);
            }
        }));
        Some(target)
    }

    pub fn has_focus(&self) -> bool {
        self.canvas.has_focus()
    }

    pub fn key(&self, press: &super::keys::KeyPress) -> bool {
        use super::keys::bindings::{
            DOWN, FIRST_ITEM, LAST_ITEM, LEFT, RIGHT, UP, VIM_DOWN, VIM_LEFT, VIM_RIGHT, VIM_UP,
        };
        if !self.has_focus() {
            return false;
        }
        if press.is(FIRST_ITEM) || press.is(LAST_ITEM) {
            let path = if press.is(FIRST_ITEM) {
                self.tiles.borrow().first().map(|(path, _)| path.clone())
            } else {
                self.tiles.borrow().last().map(|(path, _)| path.clone())
            };
            if let Some(path) = path {
                self.hovered.replace(Some(path));
                if let Some((browser, depth, position, _)) = self.target() {
                    browser.select(depth, position);
                }
                self.update_status();
                self.canvas.queue_draw();
            }
            return true;
        }
        let vim = super::preferences::PreferenceManager::shared().tenxer_mode()
            || !super::preferences::PreferenceManager::shared().type_to_search();
        let direction = if press.is(LEFT) || vim && press.is(VIM_LEFT) {
            (-1.0, 0.0)
        } else if press.is(RIGHT) || vim && press.is(VIM_RIGHT) {
            (1.0, 0.0)
        } else if press.is(UP) || vim && press.is(VIM_UP) {
            (0.0, -1.0)
        } else if press.is(DOWN) || vim && press.is(VIM_DOWN) {
            (0.0, 1.0)
        } else {
            return false;
        };
        self.move_selection(direction);
        true
    }

    pub fn focus(&self) {
        self.canvas.grab_focus();
    }

    fn hit(&self, x: f64, y: f64) -> Option<PathBuf> {
        self.tiles
            .borrow()
            .iter()
            .find(|(_, rect)| rect.contains(x, y))
            .map(|(path, _)| path.clone())
    }

    fn target(&self) -> Option<(Rc<Browser>, usize, usize, FileEntry)> {
        let path = self.hovered.borrow().clone()?;
        let browser = self.browser.upgrade()?;
        let depth = browser.active_depth()?;
        let (position, entry) = browser.entry_for_location(depth, &Location::local(path))?;
        Some((browser, depth, position, entry))
    }

    fn move_selection(&self, direction: (f64, f64)) {
        let Some(browser) = self.browser.upgrade() else {
            return;
        };
        let selected = browser
            .focused_entry()
            .and_then(|entry| entry.location.native_path().map(Path::to_path_buf));
        let tiles = self.tiles.borrow();
        let center = selected
            .as_ref()
            .and_then(|selected| tiles.iter().find(|(path, _)| path == selected))
            .map(|(_, rect)| (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0));
        let candidate = tiles
            .iter()
            .filter_map(|(path, rect)| {
                let x = rect.x + rect.width / 2.0;
                let y = rect.y + rect.height / 2.0;
                let distance = center.map_or(0.0, |(cx, cy)| {
                    let dx = x - cx;
                    let dy = y - cy;
                    if dx * direction.0 + dy * direction.1 <= 0.0 {
                        f64::INFINITY
                    } else {
                        dx.hypot(dy)
                    }
                });
                distance.is_finite().then_some((path, distance))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(path, _)| path.clone());
        drop(tiles);
        if let Some(path) = candidate {
            self.hovered.replace(Some(path));
            if let Some((browser, depth, position, _)) = self.target() {
                browser.select(depth, position);
            }
            self.update_status();
            self.canvas.queue_draw();
        }
    }

    pub fn update_selection(&self) {
        self.update_status();
        self.canvas.queue_draw();
    }

    fn update_status(&self) {
        let snapshot = self.snapshot.borrow();
        let Some(snapshot) = snapshot.as_ref() else {
            self.status
                .set_text(&crate::i18n::tr("Calculating disk usage…"));
            return;
        };
        let state = crate::i18n::tr(match snapshot.coverage {
            Coverage::Exact => "Complete",
            Coverage::Partial => "Partial",
            Coverage::Stale => "Updating…",
            Coverage::Unavailable => "Unavailable",
        });
        let selected = self
            .hovered
            .borrow()
            .as_ref()
            .and_then(|path| snapshot.children.iter().find(|child| &child.path == path));
        let position = self
            .browser
            .upgrade()
            .and_then(|browser| browser.focused_entry())
            .and_then(|entry| {
                snapshot
                    .children
                    .iter()
                    .filter(|child| child.bytes > 0)
                    .position(|child| entry.location.native_path() == Some(child.path.as_path()))
            });
        self.canvas.set_selected(position);
        let text = selected.map_or_else(
            || {
                format!(
                    "{} · {}",
                    state,
                    super::browser::format_file_size(snapshot.total)
                )
            },
            |child| {
                format!(
                    "{} · {} · {}",
                    child.path.file_name().unwrap_or_default().to_string_lossy(),
                    super::browser::format_file_size(child.bytes),
                    state
                )
            },
        );
        self.status.set_text(&text);
        super::accessibility::set_description(&self.canvas, Some(&text));
    }

    #[expect(
        deprecated,
        reason = "named semantic colors are read from the live GTK style"
    )]
    fn draw(&self, canvas: &gtk::DrawingArea, cr: &gtk::cairo::Context, width: i32, height: i32) {
        let snapshot = self.snapshot.borrow();
        let Some(snapshot) = snapshot.as_ref() else {
            return;
        };
        let style = canvas.style_context();
        let (Some(accent), Some(surface), Some(text)) = (
            style.lookup_color("strata_accent"),
            style.lookup_color("strata_surface"),
            style.lookup_color("strata_text"),
        ) else {
            return;
        };
        let children: Vec<_> = snapshot
            .children
            .iter()
            .filter(|child| child.bytes > 0)
            .collect();
        let mut weights: Vec<_> = children.iter().map(|child| child.bytes).collect();
        if snapshot.other_bytes > 0 {
            weights.push(snapshot.other_bytes);
        }
        let rectangles = layout::squarify(&weights, f64::from(width), f64::from(height));
        self.other_rect.set(if snapshot.other_bytes > 0 {
            rectangles.last().copied()
        } else {
            None
        });
        if let Some(rect) = self.other_rect.get() {
            cr.set_source_rgb(
                f64::from(surface.red()),
                f64::from(surface.green()),
                f64::from(surface.blue()),
            );
            cr.rectangle(
                rect.x + 1.0,
                rect.y + 1.0,
                (rect.width - 2.0).max(0.0),
                (rect.height - 2.0).max(0.0),
            );
            let _ = cr.fill();
            if rect.width > 60.0 && rect.height > 28.0 {
                cr.set_source_rgb(
                    f64::from(text.red()),
                    f64::from(text.green()),
                    f64::from(text.blue()),
                );
                let label = canvas.create_pango_layout(Some(&crate::i18n::tr("Other")));
                cr.move_to(rect.x + 8.0, rect.y + 7.0);
                pangocairo::functions::show_layout(cr, &label);
            }
        }
        let selected = self
            .browser
            .upgrade()
            .and_then(|browser| browser.focused_entry())
            .and_then(|entry| entry.location.native_path().map(Path::to_path_buf));
        let mut tiles = self.tiles.borrow_mut();
        tiles.clear();
        for (child, mut rect) in children.into_iter().zip(rectangles) {
            if let Some(started) = self.transition.get()
                && let Some(previous) = self.previous_tiles.borrow().get(&child.path)
            {
                let progress = (started.elapsed().as_secs_f64() / 0.14).min(1.0);
                let eased = 1.0 - (1.0 - progress).powi(3);
                rect = layout::Rect {
                    x: previous.x + (rect.x - previous.x) * eased,
                    y: previous.y + (rect.y - previous.y) * eased,
                    width: previous.width + (rect.width - previous.width) * eased,
                    height: previous.height + (rect.height - previous.height) * eased,
                };
            }
            let _ = cr.save();
            cr.rectangle(rect.x, rect.y, rect.width.max(0.0), rect.height.max(0.0));
            cr.clip();
            let active = selected.as_ref() == Some(&child.path)
                || self.hovered.borrow().as_ref() == Some(&child.path);
            let mix = if active {
                0.5
            } else if child.directory {
                0.30
            } else {
                0.15
            };
            cr.set_source_rgb(
                f64::from(surface.red()) * (1.0 - mix) + f64::from(accent.red()) * mix,
                f64::from(surface.green()) * (1.0 - mix) + f64::from(accent.green()) * mix,
                f64::from(surface.blue()) * (1.0 - mix) + f64::from(accent.blue()) * mix,
            );
            cr.rectangle(
                rect.x + 1.0,
                rect.y + 1.0,
                (rect.width - 2.0).max(0.0),
                (rect.height - 2.0).max(0.0),
            );
            let _ = cr.fill();
            if rect.width > 72.0 && rect.height > 38.0 {
                let percentage = crate::i18n::percent(
                    (child.bytes as f64 / snapshot.total.max(1) as f64 * 100.0)
                        .round()
                        .min(100.0) as usize,
                );
                let layout = canvas.create_pango_layout(Some(&format!(
                    "{}\n{} · {}",
                    child.path.file_name().unwrap_or_default().to_string_lossy(),
                    super::browser::format_file_size(child.bytes),
                    percentage
                )));
                layout.set_width(
                    ((rect.width - 16.0).max(1.0) * f64::from(gtk::pango::SCALE)) as i32,
                );
                layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
                layout.set_height(-2);
                cr.set_source_rgb(
                    f64::from(text.red()),
                    f64::from(text.green()),
                    f64::from(text.blue()),
                );
                cr.move_to(rect.x + 8.0, rect.y + 7.0);
                pangocairo::functions::show_layout(cr, &layout);
            }
            let _ = cr.restore();
            tiles.push((child.path.clone(), rect));
        }
        self.canvas
            .set_bounds(&tiles.iter().map(|(_, rect)| *rect).collect::<Vec<_>>());
    }
}

impl Drop for DiskUsageView {
    fn drop(&mut self) {
        super::collection_edit::cancel(&self.active_edits);
        if let Some(task) = self.task.get_mut().take() {
            task.abort();
        }
        if let Some(tick) = self.tick.get_mut().take() {
            tick.remove();
        }
    }
}
