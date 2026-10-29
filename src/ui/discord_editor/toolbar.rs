// SPDX-License-Identifier: MIT

use super::{Editor, model::Mode};
use gtk::prelude::*;
use std::rc::Rc;

pub(super) fn install_modes(editor: &Rc<Editor>) {
    let options = gtk::Box::new(gtk::Orientation::Vertical, 0);
    options.add_css_class("column-menu");
    let popover = crate::ui::controls::choice_popover(&options);
    editor.mode_button.add_css_class("choice-button");
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
    editor.mode_button.set_popover(Some(&popover));
}

mod selection;
pub(super) use selection::{icon_button, install_tools};
