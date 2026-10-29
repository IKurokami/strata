// SPDX-License-Identifier: MIT

use super::*;

fn option(root: &gtk::Widget, text: &str) -> Option<gtk::Button> {
    fn has_text(root: &gtk::Widget, text: &str) -> bool {
        if root
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text() == text)
        {
            return true;
        }
        let mut child = root.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if has_text(&widget, text) {
                return true;
            }
        }
        false
    }
    if let Some(button) = root.downcast_ref::<gtk::Button>()
        && has_text(root, text)
    {
        return Some(button.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(button) = option(&widget, text) {
            return Some(button);
        }
    }
    None
}

#[test]
fn choosing_language_updates_other_selectors_and_newly_built_controls() {
    crate::test_support::gtk_test(
        "ui::settings::bindings::tests::choosing_language_updates_other_selectors_and_newly_built_controls",
        || {
            let manager = PreferenceManager::shared();
            let original = manager.language();
            let running_locale = rust_i18n::locale().to_string();
            let make = || {
                choice_menu(
                    &manager,
                    "Language",
                    &crate::i18n::Language::CHOICES,
                    PreferenceManager::language,
                    PreferenceManager::set_language,
                )
            };
            let first = make();
            let second = make();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            content.append(&first);
            content.append(&second);
            let window = gtk::Window::builder().child(&content).build();
            window.present();
            first.popup();
            let popup = first.popover().expect("language choices");
            option(popup.upcast_ref(), "日本語")
                .expect("Japanese action")
                .emit_clicked();
            assert_eq!(manager.language(), crate::i18n::Language::Japanese);
            for control in [&first, &second, &make()] {
                assert_eq!(control.label().as_deref(), Some("日本語"));
            }
            assert!(!popup.is_visible(), "choosing a value dismisses the popup");
            assert_eq!(
                rust_i18n::locale().to_string(),
                running_locale,
                "language changes still require restart"
            );
            manager.set_language(original);
            window.destroy();
        },
    );
}
