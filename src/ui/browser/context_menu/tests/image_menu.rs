// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser::PeekBehavior;

#[test]
fn image_exports_are_nested_and_follow_the_selection() {
    crate::test_support::gtk_test(
        "ui::browser::context_menu::tests::image_menu::image_exports_are_nested_and_follow_the_selection",
        || {
            let manager = PreferenceManager::shared();
            manager.set_tenxer_mode(false);
            manager.set_type_to_search(false);
            let fixture = tempfile::tempdir().expect("menu fixture");
            for name in ["first.png", "second.png", "notes.txt"] {
                std::fs::write(fixture.path().join(name), []).expect("fixture file");
            }
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_view_mode(BrowserMode::Columns);
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(1000)
                .default_height(850)
                .build();
            window.present();
            view.browser().navigate(Location::local(fixture.path()));
            wait_until(|| label(&view.widget(), "notes.txt").is_some());

            let single = open_menu(&view, Some("first.png"));
            assert_image_commands(&single, &["Convert image to WebP", "Convert image to JPEG"]);
            single.popdown();
            wait_until(|| !single.is_mapped());

            view.browser()
                .select_entries_by_name(&["first.png".into(), "second.png".into()]);
            let batch = open_menu(&view, Some("first.png"));
            assert_image_commands(
                &batch,
                &[
                    "Convert images to WebP",
                    "Convert images to JPEG",
                    "Create PNG contact sheet",
                ],
            );
            batch.popdown();
            wait_until(|| !batch.is_mapped());

            let text = open_menu(&view, Some("notes.txt"));
            assert!(label(text.upcast_ref(), "Images").is_none());
            text.popdown();
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

fn assert_image_commands(menu: &gtk::Popover, expected: &[&str]) {
    let widgets = descendants(menu.upcast_ref());
    let labels: Vec<_> = widgets
        .iter()
        .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
        .map(|label| label.text())
        .collect();
    assert!(labels.iter().any(|label| label == "Images"), "{labels:?}");
    let mut commands = Vec::new();
    for widget in widgets {
        let Ok(label) = widget.downcast::<gtk::Label>() else {
            continue;
        };
        let text = label.text();
        if !text.starts_with("Convert image") && text != "Create PNG contact sheet" {
            continue;
        }
        let mut parent = Some(label.upcast::<gtk::Widget>());
        while let Some(widget) = parent {
            if let Some(submenu) = widget.downcast_ref::<gtk::Popover>() {
                assert_ne!(submenu, menu, "{text} should be nested");
                commands.push(text.to_string());
                break;
            }
            if !widget.get_visible() {
                break;
            }
            parent = widget.parent();
        }
    }
    assert_eq!(commands, expected);
}
