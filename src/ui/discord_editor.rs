// SPDX-License-Identifier: MIT

use super::{controls, keys, modal};
use gtk::{glib, prelude::*};
use model::{Document, MarkdownAction, Mode, Style};
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    path::PathBuf,
    rc::Rc,
};

mod buffer;
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

struct Editor {
    content: gtk::Box,
    layer: glib::WeakRef<gtk::Box>,
    overlay: glib::WeakRef<gtk::Overlay>,
    blurred_root: Option<glib::WeakRef<super::blur::BlurBin>>,
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
    dirty: Cell<bool>,
    saved_output: RefCell<String>,
    render_task: RefCell<Option<glib::SourceId>>,
    count: gtk::Label,
}

fn button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(&crate::i18n::tr(label));
    button.add_css_class("action-dialog-cancel");
    button
}

pub(super) fn open(parent: &impl IsA<gtk::Widget>, path: PathBuf) {
    let _ = show(parent, path);
}

fn show(parent: &impl IsA<gtk::Widget>, path: PathBuf) -> Option<Rc<Editor>> {
    let host = modal::ModalHost::blurred_for(parent)?;
    let layout = controls::modal_layout(
        crate::assets::icons::PENCIL,
        &crate::i18n::tr("Discord editor"),
        "",
        &crate::i18n::tr("Save"),
    );
    layout.subtitle.set_visible(false);
    layout.content.add_css_class("discord-editor");
    layout.content.set_size_request(780, -1);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let mode_button = gtk::MenuButton::builder().label("ANSI").build();
    mode_button.add_css_class("settings-action-library-button");
    super::accessibility::set_label(&mode_button, &crate::i18n::tr("Format"));
    let language = controls::form_entry();
    language.set_placeholder_text(Some(&crate::i18n::tr("Code language")));
    language.set_max_length(32);
    super::accessibility::set_label(&language, &crate::i18n::tr("Code language"));
    top.append(&mode_button);
    top.append(&language);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    top.append(&spacer);
    let undo = button("Undo");
    let redo = button("Redo");
    let preview = gtk::ToggleButton::with_label(&crate::i18n::tr("Preview"));
    preview.add_css_class("action-dialog-cancel");
    top.append(&undo);
    top.append(&redo);
    top.append(&preview);
    layout.body.append(&top);
    let tools = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(4)
        .row_spacing(4)
        .max_children_per_line(16)
        .build();
    layout.body.append(&tools);
    let rich = buffer::RichBuffer::default();
    rich.buffer.set_enable_undo(false);
    let view = gtk::TextView::builder()
        .buffer(&rich.buffer)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(12)
        .right_margin(12)
        .top_margin(12)
        .bottom_margin(12)
        .vexpand(true)
        .build();
    view.add_css_class("preview-text");
    super::accessibility::set_label(&view, &crate::i18n::tr("Message"));
    let display = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(12)
        .right_margin(12)
        .top_margin(12)
        .bottom_margin(12)
        .build();
    display.add_css_class("preview-text");
    super::accessibility::set_label(&display, &crate::i18n::tr("Preview"));
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.set_size_request(-1, 350);
    for (name, child) in [("edit", &view), ("preview", &display)] {
        let scroll = gtk::ScrolledWindow::builder()
            .child(child)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .build();
        stack.add_named(&scroll, Some(name));
    }
    layout.body.append(&stack);
    let status = controls::form_error_label();
    status.set_wrap(true);
    layout.body.append(&status);
    let count = gtk::Label::new(None);
    count.add_css_class("preview-note");
    count.set_xalign(0.0);
    layout.body.append(&count);
    let copy = button("Copy");
    layout.actions.prepend(&copy);
    let discard = button("Discard changes");
    discard.set_visible(false);
    layout.actions.prepend(&discard);
    layout.cancel.set_label(&crate::i18n::tr("Close"));
    let editor = Rc::new(Editor {
        content: layout.content.clone(),
        layer: glib::WeakRef::new(),
        overlay: host.overlay.downgrade(),
        blurred_root: host.blurred_root.as_ref().map(|root| root.downgrade()),
        rich,
        view,
        display,
        stack,
        mode: Cell::new(Mode::Markdown),
        mode_button,
        language,
        tools,
        status,
        save: layout.confirm.clone(),
        copy,
        discard,
        undo,
        redo,
        preview,
        file: RefCell::new(None),
        history: RefCell::new(Vec::new()),
        position: Cell::new(0),
        updating: Cell::new(false),
        busy: Cell::new(true),
        dirty: Cell::new(false),
        saved_output: RefCell::new(String::new()),
        render_task: RefCell::new(None),
        count,
    });
    let weak = Rc::downgrade(&editor);
    let layer = modal::modal_layer_with_backdrop(
        &layout.content,
        Rc::new(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.close(false);
            }
        }),
    );
    editor.layer.set(Some(&layer));
    modal::remember_modal_focus(&layer, &host.overlay);
    host.overlay.add_overlay(&layer);
    let keep = editor.clone();
    layer.connect_unrealize(move |_| {
        if let Some(task) = keep.render_task.borrow_mut().take() {
            task.remove();
        }
    });
    for button in [&layout.cancel, &layout.close] {
        let weak = Rc::downgrade(&editor);
        button.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.close(false);
            }
        });
    }
    let weak = Rc::downgrade(&editor);
    editor.discard.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            editor.close(true);
        }
    });
    editor.connect();
    editor.refresh_controls();
    layout.set_loading(true, None);
    let weak = Rc::downgrade(&editor);
    glib::spawn_future_local(async move {
        let result = storage::load(path).await;
        let Some(editor) = weak.upgrade().filter(|editor| {
            editor
                .layer
                .upgrade()
                .is_some_and(|layer| layer.is_sensitive())
        }) else {
            return;
        };
        layout.set_loading(false, None);
        editor.busy.set(false);
        match result {
            Ok(file) => match model::initial(&file.text) {
                Ok((mode, document, mut language)) => {
                    if language.is_empty() {
                        language = file
                            .path
                            .extension()
                            .and_then(|value| value.to_str())
                            .unwrap_or("")
                            .to_owned();
                    }
                    editor.file.replace(Some(file));
                    editor.restore(&Snapshot {
                        mode,
                        document,
                        language,
                    });
                    editor
                        .saved_output
                        .replace(editor.output().unwrap_or_default());
                    editor.record();
                    editor.view.grab_focus();
                }
                Err(message) => editor.error(message),
            },
            Err(error) => editor.error(error.message()),
        }
        editor.refresh_controls();
    });
    Some(editor)
}

impl Editor {
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
        if self.busy.get() {
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
        if self.render_task.borrow().is_some() {
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
        let ready = !self.busy.get() && self.file.borrow().is_some();
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
    }
    fn close(&self, discard: bool) {
        if self.busy.get() && self.file.borrow().is_some() {
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
        if let (Some(layer), Some(overlay)) = (self.layer.upgrade(), self.overlay.upgrade()) {
            let blurred = self.blurred_root.as_ref().and_then(|root| root.upgrade());
            modal::dismiss_modal_layer(&layer, &overlay, blurred.as_ref());
        }
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
        if self.busy.get() {
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
        self.refresh_controls();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = storage::save(file, text).await;
            let Some(editor) = weak.upgrade() else {
                return;
            };
            editor.busy.set(false);
            match result {
                Ok(file) => {
                    editor.saved_output.replace(file.text.clone());
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
