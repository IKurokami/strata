// SPDX-License-Identifier: MIT

use super::super::{
    Editor, button,
    color_picker::Picker,
    model::{self, Color, Mode, Style},
};
use crate::assets::icons;
use gtk::prelude::*;
use std::rc::Rc;

pub(in crate::ui::discord_editor) fn icon_button(label: &str, icon: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("discord-format-button");
    button.set_focus_on_click(false);
    let label = crate::i18n::tr(label);
    button.set_tooltip_text(Some(&label));
    crate::ui::accessibility::set_label(&button, &label);
    button.set_child(Some(&crate::assets::primary_icon(icon, 14)));
    button
}

fn menu_button(label: &str, icon: &str) -> gtk::MenuButton {
    let menu = gtk::MenuButton::new();
    menu.add_css_class("discord-format-menu");
    let label = crate::i18n::tr(label);
    menu.set_tooltip_text(Some(&label));
    crate::ui::accessibility::set_label(&menu, &label);
    menu.set_child(Some(&crate::assets::primary_icon(icon, 14)));
    menu
}

pub(in crate::ui::discord_editor) fn install_tools(editor: &Rc<Editor>) {
    while let Some(child) = editor.tools.first_child() {
        editor.tools.remove(&child);
    }
    if editor.mode.get() == Mode::Ansi {
        for (label, icon, kind) in [
            ("Bold", icons::BOLD, 0),
            ("Italic", icons::ITALIC, 1),
            ("Underline", icons::UNDERLINE, 2),
            ("Strikethrough", icons::STRIKETHROUGH, 3),
            ("Reset", icons::REMOVE_FORMATTING, 4),
        ] {
            let action = icon_button(label, icon);
            let weak = Rc::downgrade(editor);
            action.connect_clicked(move |_| {
                if let Some(editor) = weak.upgrade() {
                    let doc = editor.rich.document();
                    let mut all = true;
                    let _ = doc.edit(editor.selection(), |style, _, _| {
                        all &= match kind {
                            0 => style.bold,
                            1 => style.italic,
                            2 => style.underline,
                            3 => style.strike,
                            _ => false,
                        }
                    });
                    editor.style(|style, _, _| match kind {
                        0 => style.bold = !all,
                        1 => style.italic = !all,
                        2 => style.underline = !all,
                        3 => style.strike = !all,
                        _ => *style = Style::default(),
                    });
                }
            });
            editor.tools.insert(&action, -1);
        }
        editor.tools.insert(&color_menu(editor, false), -1);
        editor.tools.insert(&color_menu(editor, true), -1);
        editor.tools.insert(&gradient_menu(editor), -1);
    } else if editor.mode.get() == Mode::Markdown {
        for ((label, action), icon) in model::MARKDOWN_TOOLS[..4].iter().zip([
            icons::BOLD,
            icons::ITALIC,
            icons::UNDERLINE,
            icons::STRIKETHROUGH,
        ]) {
            let button = icon_button(label, icon);
            let action = *action;
            let weak = Rc::downgrade(editor);
            button.connect_clicked(move |_| {
                if let Some(editor) = weak.upgrade() {
                    editor.markdown(action);
                }
            });
            editor.tools.insert(&button, -1);
        }
        let menu = menu_button("Format", icons::SLIDERS);
        let popover = gtk::Popover::new();
        popover.add_css_class("column-popover");
        popover.add_css_class("discord-color-popover");
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
            .max_content_height(500)
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
    let menu = menu_button(
        label,
        if background {
            icons::PAINT_BUCKET
        } else {
            icons::PALETTE
        },
    );
    let popover = gtk::Popover::new();
    popover.add_css_class("column-popover");
    popover.add_css_class("discord-color-popover");
    let options = gtk::Box::new(gtk::Orientation::Vertical, 5);
    options.add_css_class("column-menu");
    let picker = Picker::new(
        label,
        if background {
            Color(43, 45, 49)
        } else {
            model::PALETTE[6]
        },
    );
    options.append(&picker.widget);
    let preview = gtk::DrawingArea::builder().content_height(20).build();
    let value = Rc::new(std::cell::Cell::new(picker.color()));
    let color = value.clone();
    preview.set_draw_func(move |_, cr, _, _| {
        let Color(r, g, b) = color.get();
        cr.set_source_rgb(
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
        );
        let _ = cr.paint();
    });
    let weak_preview = preview.downgrade();
    picker.connect_changed(move |color| {
        value.set(color);
        if let Some(preview) = weak_preview.upgrade() {
            preview.queue_draw();
        }
    });
    options.append(&preview);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let reset = button("Reset");
    let apply = button("Apply");
    actions.append(&reset);
    actions.append(&apply);
    options.append(&actions);
    let weak = Rc::downgrade(editor);
    let pop = popover.downgrade();
    apply.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            let color = picker.color();
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
    let menu = menu_button("Gradient", icons::BLEND);
    let popover = gtk::Popover::new();
    popover.add_css_class("column-popover");
    popover.add_css_class("discord-color-popover");
    let column = gtk::Box::new(gtk::Orientation::Vertical, 5);
    column.add_css_class("column-menu");
    let preview = gtk::DrawingArea::builder().content_height(20).build();
    let endpoints = Rc::new(std::cell::Cell::new([
        Color(245, 251, 255),
        Color(84, 183, 243),
    ]));
    let colors = endpoints.clone();
    preview.set_draw_func(move |_, cr, width, _| {
        let gradient = gtk::cairo::LinearGradient::new(0.0, 0.0, f64::from(width), 0.0);
        for (index, Color(r, g, b)) in colors.get().into_iter().enumerate() {
            gradient.add_color_stop_rgb(
                index as f64,
                f64::from(r) / 255.0,
                f64::from(g) / 255.0,
                f64::from(b) / 255.0,
            );
        }
        let _ = cr.set_source(&gradient);
        let _ = cr.paint();
    });
    column.append(&preview);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let pickers = ["Start color", "End color"].map(|label| {
        let index = if label == "Start color" { 0 } else { 1 };
        let field = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let heading = gtk::Label::new(Some(&crate::i18n::tr(label)));
        heading.set_xalign(0.0);
        field.append(&heading);
        let picker = Picker::new(label, endpoints.get()[index]);
        field.append(&picker.widget);
        row.append(&field);
        let endpoints = endpoints.clone();
        let weak = preview.downgrade();
        picker.connect_changed(move |color| {
            let mut values = endpoints.get();
            values[index] = color;
            endpoints.set(values);
            if let Some(preview) = weak.upgrade() {
                preview.queue_draw();
            }
        });
        picker
    });
    column.append(&row);
    let apply = button("Apply");
    column.append(&apply);
    let weak = Rc::downgrade(editor);
    let pop = popover.downgrade();
    apply.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            let [start, end] = pickers.each_ref().map(Picker::color);
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
        }
        if let Some(pop) = pop.upgrade() {
            pop.popdown();
        }
    });
    popover.set_child(Some(&column));
    menu.set_popover(Some(&popover));
    menu
}
