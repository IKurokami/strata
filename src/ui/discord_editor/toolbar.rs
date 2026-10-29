// SPDX-License-Identifier: MIT

use super::{
    Editor, button,
    model::{self, Color, Mode, Style},
};
use gtk::prelude::*;
use std::rc::Rc;

pub(super) fn install_modes(editor: &Rc<Editor>) {
    let popover = gtk::Popover::new();
    popover.add_css_class("column-popover");
    let options = gtk::Box::new(gtk::Orientation::Vertical, 0);
    options.add_css_class("column-menu");
    let mut checks = Vec::new();
    for mode in Mode::ALL {
        let (button, check) =
            crate::ui::controls::menu_option(&crate::i18n::tr(mode.label()), false);
        checks.push((mode, check));
        let weak = Rc::downgrade(editor);
        let menu = popover.downgrade();
        button.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.change_mode(mode);
            }
            if let Some(menu) = menu.upgrade() {
                menu.popdown();
            }
        });
        options.append(&button);
    }
    let weak = Rc::downgrade(editor);
    popover.connect_show(move |_| {
        if let Some(editor) = weak.upgrade() {
            for (mode, check) in &checks {
                check.set_visible(*mode == editor.mode.get());
            }
        }
    });
    popover.set_child(Some(&options));
    editor.mode_button.set_popover(Some(&popover));
}

pub(super) fn install_tools(editor: &Rc<Editor>) {
    editor
        .tools
        .set_visible(matches!(editor.mode.get(), Mode::Ansi | Mode::Markdown));
    while let Some(child) = editor.tools.first_child() {
        editor.tools.remove(&child);
    }
    if editor.mode.get() == Mode::Ansi {
        for (label, kind) in [("Bold", 0), ("Underline", 1), ("Reset", 2)] {
            let action = button(label);
            let weak = Rc::downgrade(editor);
            action.connect_clicked(move |_| {
                if let Some(editor) = weak.upgrade() {
                    let doc = editor.rich.document();
                    let selection = editor.selection();
                    let mut all = true;
                    let _ = doc.edit(selection, |style, _, _| {
                        all &= if kind == 0 {
                            style.bold
                        } else {
                            style.underline
                        }
                    });
                    editor.style(|style, _, _| match kind {
                        0 => style.bold = !all,
                        1 => style.underline = !all,
                        _ => *style = Style::default(),
                    });
                }
            });
            editor.tools.insert(&action, -1);
        }
        for background in [false, true] {
            editor.tools.insert(&color_menu(editor, background), -1);
        }
        editor.tools.insert(&gradient_menu(editor), -1);
    } else if editor.mode.get() == Mode::Markdown {
        for &(label, action) in &model::MARKDOWN_TOOLS[..4] {
            let button = button(label);
            let weak = Rc::downgrade(editor);
            button.connect_clicked(move |_| {
                if let Some(editor) = weak.upgrade() {
                    editor.markdown(action);
                }
            });
            editor.tools.insert(&button, -1);
        }
        let menu = gtk::MenuButton::builder()
            .label(crate::i18n::tr("Format"))
            .build();
        menu.add_css_class("settings-action-library-button");
        let popover = gtk::Popover::new();
        popover.add_css_class("column-popover");
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("column-menu");
        for &(label, action) in &model::MARKDOWN_TOOLS[4..] {
            let (button, _) = crate::ui::controls::menu_option(&crate::i18n::tr(label), false);
            let weak = Rc::downgrade(editor);
            let pop = popover.downgrade();
            button.connect_clicked(move |_| {
                if let Some(editor) = weak.upgrade() {
                    editor.markdown(action);
                }
                if let Some(pop) = pop.upgrade() {
                    pop.popdown();
                }
            });
            list.append(&button);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(560)
            .propagate_natural_height(true)
            .build();
        popover.set_child(Some(&scroll));
        menu.set_popover(Some(&popover));
        editor.tools.insert(&menu, -1);
    }
}

fn color_menu(editor: &Rc<Editor>, background: bool) -> gtk::MenuButton {
    let label = if background {
        "Background"
    } else {
        "Text color"
    };
    let menu = gtk::MenuButton::builder()
        .label(crate::i18n::tr(label))
        .build();
    menu.add_css_class("settings-action-library-button");
    let popover = gtk::Popover::new();
    popover.add_css_class("column-popover");
    let options = gtk::Box::new(gtk::Orientation::Vertical, 8);
    options.add_css_class("column-menu");
    let palette = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(4)
        .max_children_per_line(4)
        .build();
    for color in model::PALETTE {
        let swatch = gtk::Button::new();
        swatch.add_css_class("action-dialog-cancel");
        // The swatch represents the exported document color, not interface chrome.
        let mark = gtk::Label::new(None);
        mark.set_markup(&format!("<span foreground=\"{}\">●</span>", color.hex()));
        swatch.set_child(Some(&mark));
        crate::ui::accessibility::set_label(&swatch, &color.hex());
        let weak = Rc::downgrade(editor);
        let pop = popover.downgrade();
        swatch.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.style(|style, _, _| {
                    if background {
                        style.background = Some(color)
                    } else {
                        style.foreground = Some(color)
                    }
                });
            }
            if let Some(pop) = pop.upgrade() {
                pop.popdown();
            }
        });
        palette.insert(&swatch, -1);
    }
    options.append(&palette);
    let value = crate::ui::controls::form_entry();
    value.set_placeholder_text(Some("#RRGGBB"));
    value.set_max_length(7);
    crate::ui::accessibility::set_label(&value, &crate::i18n::tr("RGB color"));
    options.append(&value);
    let apply = button("Apply");
    let reset = button("Reset");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.append(&reset);
    row.append(&apply);
    options.append(&row);
    let weak = Rc::downgrade(editor);
    let pop = popover.downgrade();
    apply.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            if let Some(color) = Color::parse(&value.text()) {
                editor.style(|style, _, _| {
                    if background {
                        style.background = Some(color)
                    } else {
                        style.foreground = Some(color)
                    }
                });
                if let Some(pop) = pop.upgrade() {
                    pop.popdown();
                }
            } else {
                editor.error("Invalid RGB color");
            }
        }
    });
    let weak = Rc::downgrade(editor);
    let pop = popover.downgrade();
    reset.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            editor.style(|style, _, _| {
                if background {
                    style.background = None
                } else {
                    style.foreground = None
                }
            });
        }
        if let Some(pop) = pop.upgrade() {
            pop.popdown();
        }
    });
    popover.set_child(Some(&options));
    menu.set_popover(Some(&popover));
    menu
}

fn gradient_menu(editor: &Rc<Editor>) -> gtk::MenuButton {
    let menu = gtk::MenuButton::builder()
        .label(crate::i18n::tr("Gradient"))
        .build();
    menu.add_css_class("settings-action-library-button");
    let popover = gtk::Popover::new();
    popover.add_css_class("column-popover");
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.add_css_class("column-menu");
    let fields = [("Start color", "#f5fbff"), ("End color", "#54b7f3")].map(|(label, value)| {
        let entry = crate::ui::controls::form_entry();
        entry.set_placeholder_text(Some(&crate::i18n::tr(label)));
        entry.set_text(value);
        entry.set_max_length(7);
        crate::ui::accessibility::set_label(&entry, &crate::i18n::tr(label));
        column.append(&entry);
        entry
    });
    let apply = button("Apply");
    column.append(&apply);
    let weak = Rc::downgrade(editor);
    let pop = popover.downgrade();
    apply.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            let [Some(start), Some(end)] =
                fields.each_ref().map(|field| Color::parse(&field.text()))
            else {
                editor.error("Invalid RGB color");
                return;
            };
            editor.style(|style, index, count| {
                let mix = |a: u8, b: u8| {
                    ((f64::from(a)
                        + (f64::from(b) - f64::from(a)) * index as f64
                            / count.saturating_sub(1).max(1) as f64)
                        .round()) as u8
                };
                style.foreground = Some(Color(
                    mix(start.0, end.0),
                    mix(start.1, end.1),
                    mix(start.2, end.2),
                ));
            });
            if let Some(pop) = pop.upgrade() {
                pop.popdown();
            }
        }
    });
    popover.set_child(Some(&column));
    menu.set_popover(Some(&popover));
    menu
}
