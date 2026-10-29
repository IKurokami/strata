// SPDX-License-Identifier: MIT

use super::*;

fn wait(mut condition: impl FnMut() -> bool) {
    let started = std::time::Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(8),
            "editor operation timed out"
        );
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn ansi_editing_undo_mode_changes_and_atomic_save_preserve_the_source() {
    crate::test_support::gtk_test(
        "ui::discord_editor::tests::ansi_editing_undo_mode_changes_and_atomic_save_preserve_the_source",
        || {
            let folder = tempfile::tempdir().expect("fixture");
            let path = folder.path().join("message.txt");
            std::fs::write(&path, "ảnh Việt").expect("source");
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
            let window = gtk::Window::builder()
                .default_width(900)
                .default_height(600)
                .child(&overlay)
                .build();
            window.present();
            let editor = Editor::new(Some(path.clone()), "ảnh Việt");
            overlay.set_child(Some(&editor.widget()));
            editor.start_editing();
            wait(|| !editor.busy.get());
            assert_eq!(editor.mode.get(), Mode::Markdown);
            assert!(!editor.dirty.get());
            editor.change_mode(Mode::Ansi);
            assert!(editor.output().expect("ANSI").starts_with("```ansi\n"));
            assert_eq!(
                std::fs::read_to_string(&path).expect("unchanged"),
                "ảnh Việt"
            );
            editor.select(0..3);
            wait(|| editor.format_popup.is_visible());
            editor.selecting.set(true);
            editor.show_selection_tools();
            assert!(!editor.format_popup.is_visible());
            editor.selecting.set(false);
            editor.show_selection_tools();
            assert!(editor.format_popup.is_visible());
            editor.style(|style, _, _| {
                style.bold = true;
                style.foreground = Some(model::Color(12, 34, 56));
            });
            let styled = editor.snapshot();
            assert!(styled.document.runs[0].style.bold);
            editor.history_step(false);
            assert!(!editor.rich.document().runs[0].style.bold);
            editor.history_step(true);
            assert_eq!(editor.snapshot(), styled);
            editor.change_mode(Mode::Markdown);
            assert_eq!(editor.output().expect("plain body"), "ảnh Việt");
            editor.history_step(false);
            assert_eq!(editor.snapshot(), styled);
            editor.preview.set_active(true);
            assert_eq!(
                editor.display.buffer().text(
                    &editor.display.buffer().start_iter(),
                    &editor.display.buffer().end_iter(),
                    true
                ),
                "ảnh Việt"
            );
            editor.preview.set_active(false);
            editor
                .rich
                .buffer
                .insert_at_cursor(&"x".repeat(model::BYTE_LIMIT + 1));
            assert_eq!(editor.rich.document().text(), "ảnh Việt");
            editor.close(false);
            assert!(editor.discard.is_visible());
            assert!(editor.is_editing());
            editor.save();
            wait(|| !editor.busy.get());
            let saved = std::fs::read_to_string(&path).expect("saved");
            let (_, document, _) = model::initial(&saved).expect("readable ANSI");
            assert_eq!(document, styled.document);
            assert!(!editor.dirty.get());
            editor.close(false);
            assert!(!editor.is_editing());
            assert!(!editor.text_view().is_editable());
            window.close();
        },
    );
}

#[test]
fn closing_during_load_discards_late_results_and_markdown_preview_is_bounded() {
    crate::test_support::gtk_test(
        "ui::discord_editor::tests::closing_during_load_discards_late_results_and_markdown_preview_is_bounded",
        || {
            let folder = tempfile::tempdir().expect("fixture");
            let path = folder.path().join("message.txt");
            std::fs::write(&path, "unchanged").expect("source");
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
            let window = gtk::Window::builder()
                .default_width(900)
                .default_height(600)
                .child(&overlay)
                .build();
            window.present();
            let editor = Editor::new(Some(path.clone()), "ảnh Việt");
            overlay.set_child(Some(&editor.widget()));
            editor.start_editing();
            editor.close(true);
            let until = std::time::Instant::now() + std::time::Duration::from_millis(300);
            while std::time::Instant::now() < until {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
            }
            assert!(editor.file.borrow().is_none());
            assert_eq!(
                std::fs::read_to_string(path).expect("unchanged"),
                "unchanged"
            );
            let preview = markdown::render(
                "# title\n__**bold**__ ~~gone~~ ||secret|| [site](https://example.com)\n```rs\n**literal**\n```",
            );
            assert_eq!(
                preview.text(),
                "title\nbold gone ▇▇▇▇▇▇ site\n**literal**\n"
            );
            assert!(
                preview
                    .runs
                    .iter()
                    .any(|run| run.text == "bold" && run.style.bold && run.style.underline)
            );
            window.close();
        },
    );
}
