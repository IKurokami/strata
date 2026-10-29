// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn create(path: Option<PathBuf>, source: &str) -> Rc<Editor> {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 3);
    content.add_css_class("discord-editor");
    content.set_vexpand(true);
    let top = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(3)
        .row_spacing(2)
        .max_children_per_line(3)
        .build();
    top.add_css_class("discord-editor-controls");
    let mode_button = gtk::MenuButton::builder().label("ANSI").build();
    mode_button.add_css_class("settings-action-library-button");
    mode_button.add_css_class("discord-compact-menu");
    super::super::accessibility::set_label(&mode_button, &crate::i18n::tr("Format"));
    let language = super::super::controls::form_entry();
    language.set_placeholder_text(Some(&crate::i18n::tr("Code language")));
    language.set_max_length(32);
    super::super::accessibility::set_label(&language, &crate::i18n::tr("Code language"));
    let preview = gtk::ToggleButton::new();
    let preview_label = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    preview_label.append(&crate::assets::primary_icon(crate::assets::icons::EYE, 14));
    preview_label.append(&gtk::Label::new(Some(&crate::i18n::tr("Preview"))));
    preview.set_child(Some(&preview_label));
    super::super::accessibility::set_label(&preview, &crate::i18n::tr("Preview"));
    preview.add_css_class("action-dialog-cancel");
    preview.add_css_class("discord-compact-action");
    top.insert(&mode_button, -1);
    top.insert(&language, -1);
    top.insert(&preview, -1);
    content.append(&top);
    let rich = buffer::RichBuffer::default();
    rich.buffer.set_enable_undo(false);
    let view = gtk::TextView::builder()
        .buffer(&rich.buffer)
        .editable(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(8)
        .right_margin(8)
        .top_margin(8)
        .bottom_margin(8)
        .vexpand(true)
        .build();
    view.add_css_class("preview-text");
    super::super::accessibility::set_label(&view, &crate::i18n::tr("Message"));
    let display = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .left_margin(8)
        .right_margin(8)
        .top_margin(8)
        .bottom_margin(8)
        .build();
    display.add_css_class("preview-text");
    super::super::accessibility::set_label(&display, &crate::i18n::tr("Preview"));
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    for (name, child) in [("edit", &view), ("preview", &display)] {
        let scroll = gtk::ScrolledWindow::builder()
            .child(child)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .build();
        stack.add_named(&scroll, Some(name));
    }
    content.append(&stack);
    let tools = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(2)
        .row_spacing(2)
        .max_children_per_line(8)
        .build();
    let format_popup = gtk::Popover::builder()
        .autohide(false)
        .has_arrow(false)
        .position(gtk::PositionType::Top)
        .child(&tools)
        .build();
    format_popup.add_css_class("column-popover");
    format_popup.add_css_class("discord-format-popover");
    tools.add_css_class("discord-selection-tools");
    format_popup.set_parent(&view);
    let status = super::super::controls::form_error_label();
    status.set_wrap(true);
    content.append(&status);
    let actions = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .column_spacing(3)
        .row_spacing(2)
        .max_children_per_line(6)
        .build();
    actions.add_css_class("discord-editor-controls");
    let undo = toolbar::icon_button("Undo", crate::assets::icons::UNDO_2);
    let redo = toolbar::icon_button("Redo", crate::assets::icons::REDO_2);
    let copy = button("Copy");
    let save = button("Save");
    let discard = button("Discard changes");
    let close = toolbar::icon_button("Close", crate::assets::icons::X);
    actions.insert(&undo, -1);
    actions.insert(&redo, -1);
    actions.insert(&copy, -1);
    actions.insert(&save, -1);
    actions.insert(&close, -1);
    actions.insert(&discard, -1);
    content.append(&actions);
    let count = gtk::Label::new(None);
    count.add_css_class("preview-note");
    count.add_css_class("discord-editor-count");
    count.set_xalign(0.0);
    content.append(&count);
    let editor = Rc::new(Editor {
        content,
        path,
        top,
        actions,
        format_popup,
        pointer: Cell::new(None),
        selecting: Cell::new(false),
        selection_task: RefCell::new(None),
        editing: Cell::new(false),
        attached: Cell::new(true),
        epoch: Cell::new(0),
        saved_snapshot: RefCell::new(None),
        rich,
        view,
        display,
        stack,
        mode: Cell::new(Mode::Markdown),
        mode_button,
        language,
        tools,
        status,
        save,
        copy,
        discard,
        undo,
        redo,
        preview,
        file: RefCell::new(None),
        history: RefCell::new(Vec::new()),
        position: Cell::new(0),
        updating: Cell::new(false),
        busy: Cell::new(false),
        saving: Cell::new(false),
        dirty: Cell::new(false),
        saved_output: RefCell::new(String::new()),
        render_task: RefCell::new(None),
        count,
        surface_changed: RefCell::new(None),
    });
    editor.connect();
    editor.connect_selection();
    let (mode, document, language) = model::initial(source)
        .unwrap_or_else(|_| (Mode::Markdown, Document::plain(source), String::new()));
    editor.restore(&Snapshot {
        mode,
        document,
        language,
    });
    editor.refresh_controls();
    let weak = Rc::downgrade(&editor);
    close.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            editor.close(false);
        }
    });
    let weak = Rc::downgrade(&editor);
    editor.discard.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            editor.close(true);
        }
    });
    editor
}
