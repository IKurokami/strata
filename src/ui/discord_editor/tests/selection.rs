// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn ending_selection_reveals_tools_even_when_marks_changed_before_release() {
    crate::test_support::gtk_test(
        "ui::discord_editor::tests::selection::ending_selection_reveals_tools_even_when_marks_changed_before_release",
        || {
            let directory = tempfile::tempdir().expect("fixture");
            let path = directory.path().join("message.txt");
            let source = "Select this Vietnamese text: ảnh Việt, màu sắc.";
            std::fs::write(&path, source).expect("source");
            let editor = Editor::new(Some(path.clone()), source);
            let window = gtk::Window::builder()
                .default_width(900)
                .default_height(600)
                .child(&editor.widget())
                .build();
            window.present();
            editor.start_editing();
            wait(|| !editor.busy.get() && editor.view.is_mapped());
            for mode in [Mode::Markdown, Mode::Ansi] {
                editor.change_mode(mode);
                editor.begin_selection();
                editor.select(2..18);
                wait(|| editor.selection_task.borrow().is_none());
                assert!(
                    !editor.format_popup.is_mapped(),
                    "selection changes during a drag cannot open tools"
                );
                editor.end_selection();
                wait(|| editor.format_popup.is_mapped());
                assert!(
                    editor.rich.buffer.has_selection(),
                    "opening tools retains the selected text"
                );
                editor.select(25..25);
                wait(|| !editor.format_popup.is_mapped());
            }
            editor.select(2..18);
            wait(|| editor.format_popup.is_mapped());
            editor.suspend();
            editor.end_selection();
            wait(|| editor.selection_task.borrow().is_none());
            assert!(
                !editor.format_popup.is_mapped(),
                "a late release cannot reopen a detached editor"
            );
            assert_eq!(std::fs::read_to_string(path).expect("unchanged"), source);
            window.close();
        },
    );
}
