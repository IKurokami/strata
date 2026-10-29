// SPDX-License-Identifier: MIT

use super::keys;
use gtk::{glib, prelude::*};
use model::{Document, MarkdownAction, Mode, Style};
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    path::PathBuf,
    rc::Rc,
};

mod buffer;
mod color_picker;
mod inline;
mod markdown;
mod model;
mod storage;
#[cfg(test)]
mod tests;
mod toolbar;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    mode: Mode,
    document: Document,
    language: String,
}

pub(super) struct Editor {
    content: gtk::Box,
    path: Option<PathBuf>,
    top: gtk::FlowBox,
    actions: gtk::FlowBox,
    format_popup: gtk::Popover,
    pointer: Cell<Option<(f64, f64)>>,
    selecting: Cell<bool>,
    selection_task: RefCell<Option<glib::SourceId>>,
    editing: Cell<bool>,
    attached: Cell<bool>,
    epoch: Cell<u64>,
    saved_snapshot: RefCell<Option<Snapshot>>,
    rich: buffer::RichBuffer,
    view: gtk::TextView,
    display: gtk::TextView,
    stack: gtk::Stack,
    mode: Cell<Mode>,
    mode_button: gtk::MenuButton,
    language: gtk::Entry,
    tools: gtk::FlowBox,
    status: gtk::Label,
    save: gtk::Button,
    copy: gtk::Button,
    discard: gtk::Button,
    undo: gtk::Button,
    redo: gtk::Button,
    preview: gtk::ToggleButton,
    file: RefCell<Option<storage::FileSnapshot>>,
    history: RefCell<Vec<Snapshot>>,
    position: Cell<usize>,
    updating: Cell<bool>,
    busy: Cell<bool>,
    saving: Cell<bool>,
    dirty: Cell<bool>,
    saved_output: RefCell<String>,
    render_task: RefCell<Option<glib::SourceId>>,
    count: gtk::Label,
    surface_changed: RefCell<Option<Rc<dyn Fn()>>>,
}

fn button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(&crate::i18n::tr(label));
    button.add_css_class("action-dialog-cancel");
    button
}

impl Editor {
    pub(super) fn new(path: Option<PathBuf>, source: &str) -> Rc<Self> {
        inline::create(path, source)
    }
    pub(super) fn is_ansi(source: &str) -> bool {
        source.len() <= model::BYTE_LIMIT
            && model::initial(source).is_ok_and(|(mode, _, _)| mode == Mode::Ansi)
    }
    pub(super) fn widget(&self) -> gtk::Widget {
        self.content.clone().upcast()
    }
    pub(super) fn text_view(&self) -> gtk::TextView {
        if self.preview.is_active() {
            self.display.clone()
        } else {
            self.view.clone()
        }
    }
    pub(super) fn set_surface_changed(&self, callback: impl Fn() + 'static) {
        self.surface_changed.replace(Some(Rc::new(callback)));
    }
    pub(super) fn path(&self) -> Option<PathBuf> {
        self.path.clone()
    }
    pub(super) fn has_draft(&self) -> bool {
        self.editing.get() && (self.dirty.get() || self.saving.get())
    }
    pub(super) fn is_editing(&self) -> bool {
        self.editing.get()
    }
    pub(super) fn set_wrapped(&self, wrapped: bool) {
        let mode = if wrapped {
            gtk::WrapMode::WordChar
        } else {
            gtk::WrapMode::None
        };
        self.view.set_wrap_mode(mode);
        self.display.set_wrap_mode(mode);
    }
    pub(super) fn suspend(&self) {
        self.attached.set(false);
        self.selecting.set(false);
        self.epoch.set(self.epoch.get().wrapping_add(1));
        self.format_popup.popdown();
        if let Some(task) = self.render_task.borrow_mut().take() {
            task.remove();
        }
        if let Some(task) = self.selection_task.borrow_mut().take() {
            task.remove();
        }
        if !self.saving.get() {
            self.busy.set(false);
            if !self.dirty.get() {
                self.editing.set(false);
            }
        }
    }
    pub(super) fn resume(self: &Rc<Self>) {
        self.attached.set(true);
        self.schedule();
    }
    pub(super) fn start_editing(self: &Rc<Self>) {
        if self.editing.get() || self.busy.get() {
            return;
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        self.editing.set(true);
        self.busy.set(true);
        self.refresh_controls();
        let epoch = self.epoch.get();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = storage::load(path).await;
            let Some(editor) = weak
                .upgrade()
                .filter(|editor| editor.attached.get() && editor.epoch.get() == epoch)
            else {
                return;
            };
            editor.busy.set(false);
            match result {
                Ok(file) => {
                    let (mode, document, mut language) =
                        model::initial(&file.text).unwrap_or_else(|_| {
                            (Mode::Markdown, Document::plain(&file.text), String::new())
                        });
                    if language.is_empty() {
                        language = file
                            .path
                            .extension()
                            .and_then(|value| value.to_str())
                            .unwrap_or("")
                            .to_owned();
                    }
                    let snapshot = Snapshot {
                        mode,
                        document,
                        language,
                    };
                    editor.file.replace(Some(file));
                    editor.restore(&snapshot);
                    editor.saved_snapshot.replace(Some(snapshot));
                    editor
                        .saved_output
                        .replace(editor.output().unwrap_or_default());
                    editor.history.borrow_mut().clear();
                    editor.position.set(0);
                    editor.record();
                    editor.view.grab_focus();
                }
                Err(error) => {
                    editor.error(error.message());
                    editor.editing.set(false);
                }
            }
            editor.refresh_controls();
        });
    }
    fn connect_selection(self: &Rc<Self>) {
        let click = gtk::GestureClick::new();
        click.set_button(1);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.set_propagation_limit(gtk::PropagationLimit::SameNative);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, _, x, y| {
            if let Some(editor) = weak.upgrade() {
                editor.pointer.set(Some((x, y)));
                editor.selecting.set(true);
                editor.format_popup.popdown();
            }
        });
        let weak = Rc::downgrade(self);
        click.connect_released(move |_, _, x, y| {
            if let Some(editor) = weak.upgrade() {
                editor.pointer.set(Some((x, y)));
                editor.selecting.set(false);
                editor.queue_selection();
            }
        });
        let weak = Rc::downgrade(self);
        click.connect_cancel(move |_, _| {
            if let Some(editor) = weak.upgrade() {
                editor.selecting.set(false);
            }
        });
        self.view.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        motion.set_propagation_limit(gtk::PropagationLimit::SameNative);
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(editor) = weak.upgrade() {
                editor.pointer.set(Some((x, y)));
            }
        });
        self.view.add_controller(motion);
        let weak = Rc::downgrade(self);
        self.rich.buffer.connect_mark_set(move |_, _, _| {
            if let Some(editor) = weak.upgrade().filter(|editor| !editor.updating.get()) {
                editor.queue_selection();
            }
        });
    }
    fn queue_selection(self: &Rc<Self>) {
        if self.selection_task.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        self.selection_task
            .replace(Some(glib::idle_add_local_once(move || {
                if let Some(editor) = weak.upgrade() {
                    editor.selection_task.take();
                    editor.show_selection_tools();
                }
            })));
    }
    fn show_selection_tools(&self) {
        let allowed = self.attached.get()
            && self.editing.get()
            && !self.busy.get()
            && !self.selecting.get()
            && !self.preview.is_active()
            && !self.selection().is_empty()
            && matches!(self.mode.get(), Mode::Ansi | Mode::Markdown);
        if !allowed {
            self.format_popup.popdown();
            return;
        }
        if !self.view.is_mapped() {
            return;
        }
        let (x, y) = self.pointer.get().unwrap_or_else(|| {
            let cursor = self
                .rich
                .buffer
                .iter_at_offset(self.rich.buffer.cursor_position());
            let location = self.view.iter_location(&cursor);
            let (x, y) = self.view.buffer_to_window_coords(
                gtk::TextWindowType::Widget,
                location.x(),
                location.y(),
            );
            (f64::from(x), f64::from(y))
        });
        self.format_popup
            .set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
        self.format_popup.popup();
    }
    fn connect(self: &Rc<Self>) {
        toolbar::install_modes(self);
        let weak = Rc::downgrade(self);
        self.rich
            .buffer
            .connect_insert_text(move |buffer, _, text| {
                if let Some(editor) = weak.upgrade()
                    && !editor.updating.get()
                    && (buffer
                        .text(&buffer.start_iter(), &buffer.end_iter(), true)
                        .len()
                        + text.len()
                        > model::BYTE_LIMIT
                        || text.contains('\0'))
                {
                    buffer.stop_signal_emission_by_name("insert-text");
                    editor.error("Text is too large for Discord editing");
                }
            });
        let weak = Rc::downgrade(self);
        self.rich.buffer.connect_changed(move |_| {
            if let Some(editor) = weak.upgrade()
                && !editor.updating.get()
            {
                editor.dirty.set(true);
                editor.schedule();
            }
        });
        let weak = Rc::downgrade(self);
        self.rich.buffer.connect_end_user_action(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.record();
            }
        });
        let weak = Rc::downgrade(self);
        self.language.connect_changed(move |_| {
            if let Some(editor) = weak.upgrade()
                && !editor.updating.get()
            {
                editor.record();
                editor.schedule();
            }
        });
        let weak = Rc::downgrade(self);
        self.preview.connect_toggled(move |button| {
            if let Some(editor) = weak.upgrade() {
                editor.stack.set_visible_child_name(if button.is_active() {
                    "preview"
                } else {
                    "edit"
                });
                editor.render_preview();
                editor.refresh_controls();
                editor.format_popup.popdown();
                if let Some(changed) = editor.surface_changed.borrow().as_ref() {
                    changed();
                }
            }
        });
        for (button, redo) in [(&self.undo, false), (&self.redo, true)] {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(editor) = weak.upgrade() {
                    editor.history_step(redo);
                }
            });
        }
        let weak = Rc::downgrade(self);
        self.copy.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                match editor.output() {
                    Ok(text) => {
                        editor.content.display().clipboard().set_text(&text);
                        editor.status.set_visible(false);
                    }
                    Err(message) => editor.error(message),
                }
            }
        });
        let weak = Rc::downgrade(self);
        self.save.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.save();
            }
        });
        let controller = gtk::EventControllerKey::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys::on_press(&controller, move |_, press| {
            let Some(editor) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !editor.editing.get() {
                return glib::Propagation::Proceed;
            }
            if press.is(keys::bindings::UNDO) {
                editor.history_step(false);
            } else if press.is(keys::bindings::REDO) || press.is(keys::bindings::REDO_Y) {
                editor.history_step(true);
            } else if press.is(keys::bindings::DISCORD_SAVE) {
                editor.save();
            } else if press.is(keys::bindings::ESCAPE_ANY) {
                let in_popover = editor
                    .content
                    .root()
                    .and_downcast::<gtk::Window>()
                    .and_then(|window| gtk::prelude::RootExt::focus(&window))
                    .is_some_and(|focus| focus.ancestor(gtk::Popover::static_type()).is_some());
                if in_popover {
                    return glib::Propagation::Proceed;
                }
                editor.close(false);
            } else {
                return glib::Propagation::Proceed;
            }
            glib::Propagation::Stop
        });
        self.content.add_controller(controller);
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            mode: self.mode.get(),
            document: self.rich.document(),
            language: self.language.text().to_string(),
        }
    }
    fn selection(&self) -> Range<usize> {
        let (start, end) = self.rich.buffer.selection_bounds().unwrap_or_else(|| {
            let iter = self
                .rich
                .buffer
                .iter_at_offset(self.rich.buffer.cursor_position());
            (iter, iter)
        });
        start.offset() as usize..end.offset() as usize
    }
    fn select(&self, range: Range<usize>) {
        self.rich.buffer.select_range(
            &self.rich.buffer.iter_at_offset(range.start as i32),
            &self.rich.buffer.iter_at_offset(range.end as i32),
        );
    }
    fn output(&self) -> Result<String, &'static str> {
        model::export(
            self.mode.get(),
            &self.rich.document(),
            &self.language.text(),
        )
    }
    fn error(&self, message: &str) {
        self.status.set_text(&crate::i18n::tr(message));
        self.status.set_visible(true);
    }
    pub(super) fn show_error(&self, message: &str) {
        self.error(message);
    }
    fn restore(self: &Rc<Self>, snapshot: &Snapshot) {
        self.updating.set(true);
        self.mode.set(snapshot.mode);
        if snapshot.mode == Mode::Ansi {
            self.view.add_css_class("discord-ansi-canvas");
            self.display.add_css_class("discord-ansi-canvas");
        } else {
            self.view.remove_css_class("discord-ansi-canvas");
            self.display.remove_css_class("discord-ansi-canvas");
        }
        self.mode_button
            .set_label(&crate::i18n::tr(snapshot.mode.label()));
        self.language.set_text(&snapshot.language);
        self.rich.show(&snapshot.document);
        self.updating.set(false);
        toolbar::install_tools(self);
        self.schedule();
    }
    fn record(&self) {
        if self.updating.get() || self.file.borrow().is_none() {
            return;
        }
        let current = self.snapshot();
        let mut history = self.history.borrow_mut();
        if history.get(self.position.get()) != Some(&current) {
            history.truncate(self.position.get() + 1);
            history.push(current);
            while history.len() > 32
                || (history.len() > 1
                    && history
                        .iter()
                        .map(|snapshot| {
                            snapshot
                                .document
                                .runs
                                .iter()
                                .map(|run| run.text.len())
                                .sum::<usize>()
                                + snapshot.document.runs.len() * std::mem::size_of::<model::Run>()
                        })
                        .sum::<usize>()
                        > 4 * 1024 * 1024)
            {
                history.remove(0);
            }
            self.position.set(history.len() - 1);
        }
        drop(history);
        self.refresh_controls();
    }
    fn history_step(self: &Rc<Self>, redo: bool) {
        if !self.editing.get() || self.busy.get() {
            return;
        }
        self.record();
        let position = self.position.get();
        let length = self.history.borrow().len();
        let next = if redo {
            if position + 1 >= length {
                return;
            }
            position + 1
        } else {
            if position == 0 {
                return;
            }
            position - 1
        };
        let snapshot = self.history.borrow()[next].clone();
        self.position.set(next);
        self.restore(&snapshot);
        self.refresh_controls();
    }
    fn schedule(self: &Rc<Self>) {
        if !self.attached.get() || self.render_task.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        self.render_task.replace(Some(glib::timeout_add_local_once(
            std::time::Duration::from_millis(90),
            move || {
                if let Some(editor) = weak.upgrade() {
                    editor.render_task.take();
                    editor.record();
                    editor.render_preview();
                    editor.refresh_controls();
                }
            },
        )));
    }
    fn refresh_controls(&self) {
        let ready = self.editing.get() && !self.busy.get() && self.file.borrow().is_some();
        self.top.set_visible(self.editing.get());
        self.actions.set_visible(self.editing.get());
        self.count.set_visible(self.editing.get());
        self.view.set_editable(ready);
        self.mode_button.set_sensitive(ready);
        self.language.set_sensitive(ready);
        self.language.set_visible(self.mode.get() == Mode::Code);
        self.tools.set_sensitive(ready && !self.preview.is_active());
        let output = self.output();
        let dirty = self.file.borrow().as_ref().is_some_and(|_| {
            output
                .as_ref()
                .map_or(true, |output| output != self.saved_output.borrow().as_str())
        });
        self.dirty.set(dirty);
        self.discard.set_visible(dirty && self.editing.get());
        self.save.set_sensitive(ready && dirty && output.is_ok());
        self.copy.set_sensitive(ready && output.is_ok());
        self.undo.set_sensitive(ready && self.position.get() > 0);
        self.redo
            .set_sensitive(ready && self.position.get() + 1 < self.history.borrow().len());
        if let Ok(text) = output {
            let count = text.chars().count();
            self.count.set_text(&rust_i18n::t!(
                "%{count} / 2000 · Nitro 4000",
                count = count
            ));
        }
    }
    fn render_preview(&self) {
        if !self.preview.is_active() {
            return;
        }
        let document = if self.mode.get() == Mode::Markdown {
            markdown::render(&self.rich.document().text())
        } else {
            self.rich.document()
        };
        let rendered = buffer::RichBuffer::default();
        rendered.show(&document);
        self.display.set_buffer(Some(&rendered.buffer));
        if let Some(changed) = self.surface_changed.borrow().as_ref() {
            changed();
        }
    }
    fn close(self: &Rc<Self>, discard: bool) {
        if self.saving.get() {
            return;
        }
        self.refresh_controls();
        if self.dirty.get() && !discard {
            self.error("Unsaved changes");
            self.discard.set_visible(true);
            return;
        }
        if let Some(task) = self.render_task.borrow_mut().take() {
            task.remove();
        }
        if discard && let Some(snapshot) = self.saved_snapshot.borrow().clone() {
            self.restore(&snapshot);
        }
        self.epoch.set(self.epoch.get().wrapping_add(1));
        self.busy.set(false);
        self.editing.set(false);
        self.preview.set_active(false);
        self.format_popup.popdown();
        self.status.set_visible(false);
        self.refresh_controls();
    }
    fn change_mode(self: &Rc<Self>, mode: Mode) {
        if self.mode.get() == mode {
            return;
        }
        self.record();
        let document = self.rich.document();
        let document = if mode == Mode::Ansi {
            match model::parse_ansi(&document.text()) {
                Ok(document) => document,
                Err(message) => {
                    self.error(message);
                    return;
                }
            }
        } else {
            Document::plain(&document.text())
        };
        self.restore(&Snapshot {
            mode,
            document,
            language: self.language.text().to_string(),
        });
        self.record();
        self.status.set_visible(false);
    }
    fn style(self: &Rc<Self>, edit: impl Fn(&mut Style, usize, usize)) {
        if !self.editing.get() || self.busy.get() {
            return;
        }
        let range = self.selection();
        if range.is_empty() {
            self.error("Select text to format");
            return;
        }
        let document = self.rich.document().edit(range.clone(), edit);
        if document.runs.len() > model::RUN_LIMIT {
            self.error("Text is too large for Discord editing");
            return;
        }
        self.updating.set(true);
        self.rich.show(&document);
        self.select(range);
        self.updating.set(false);
        self.record();
        self.view.grab_focus();
        self.status.set_visible(false);
        self.schedule();
    }
    fn markdown(self: &Rc<Self>, action: MarkdownAction) {
        let (text, range) =
            model::markdown_edit(&self.rich.document().text(), self.selection(), action);
        if text.len() > model::BYTE_LIMIT {
            self.error("Text is too large for Discord editing");
            return;
        }
        self.updating.set(true);
        self.rich.show(&Document::plain(&text));
        self.select(range);
        self.updating.set(false);
        self.record();
        self.schedule();
        self.view.grab_focus();
    }
    fn save(self: &Rc<Self>) {
        if !self.editing.get() || self.busy.get() {
            return;
        }
        let text = match self.output() {
            Ok(text) => text,
            Err(message) => {
                self.error(message);
                return;
            }
        };
        let Some(file) = self.file.borrow().clone() else {
            return;
        };
        self.busy.set(true);
        self.saving.set(true);
        self.format_popup.popdown();
        self.refresh_controls();
        let snapshot = self.snapshot();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = storage::save(file, text).await;
            let Some(editor) = weak.upgrade() else {
                return;
            };
            editor.busy.set(false);
            editor.saving.set(false);
            match result {
                Ok(file) => {
                    editor.saved_output.replace(file.text.clone());
                    editor.saved_snapshot.replace(Some(snapshot));
                    editor.file.replace(Some(file));
                    editor.status.set_visible(false);
                    editor.discard.set_visible(false);
                    crate::adapters::clear_preview_cache();
                }
                Err(error) => editor.error(error.message()),
            }
            editor.refresh_controls();
        });
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        if let Some(task) = self.render_task.get_mut().take() {
            task.remove();
        }
        if let Some(task) = self.selection_task.get_mut().take() {
            task.remove();
        }
        self.format_popup.unparent();
    }
}
