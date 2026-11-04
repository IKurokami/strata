// SPDX-License-Identifier: MIT

use super::*;

fn startup_switch(page: &gtk::Widget) -> gtk::Switch {
    fn title(widget: &gtk::Widget, text: &str) -> Option<gtk::Widget> {
        if widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text() == text)
        {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(found) = title(&widget, text) {
                return Some(found);
            }
        }
        None
    }
    let mut parent = title(page, &crate::i18n::tr("Start with Windows"))
        .expect("startup setting")
        .parent();
    while let Some(row) = parent {
        let mut child = row.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(toggle) = widget.downcast::<gtk::Switch>() {
                return toggle;
            }
        }
        parent = row.parent();
    }
    panic!("startup toggle missing")
}

#[test]
fn default_startup_setting_and_tray_action_synchronize_across_settings_pages() {
    crate::test_support::gtk_test(
        "ui::settings::startup_windows_tests::default_startup_setting_and_tray_action_synchronize_across_settings_pages",
        || {
            let manager = PreferenceManager::shared();
            assert!(manager.start_with_windows(), "first-run startup default");
            let (first_page, _, _) = general_page(manager.clone());
            let (second_page, _, _) = general_page(manager.clone());
            let first = startup_switch(&first_page);
            let second = startup_switch(&second_page);
            assert!(first.is_active() && second.is_active());
            first.set_active(false);
            assert!(!manager.start_with_windows() && !second.is_active());
            crate::ui::windows_desktop::toggle_startup(&manager);
            assert!(manager.start_with_windows() && first.is_active() && second.is_active());
            let (rebuilt, _, _) = general_page(manager.clone());
            assert!(startup_switch(&rebuilt).is_active());
            second.set_active(false);
            assert!(!first.is_active() && !startup_switch(&rebuilt).is_active());
            crate::ui::preferences::flush_pending_preferences();
            assert!(
                !PreferenceManager::reload_saved_for_test().start_with_windows(),
                "disabled choice survives relaunch"
            );
        },
    );
}
