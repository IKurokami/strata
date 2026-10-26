// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn pathological_code_previews_search_safely_and_cancel_replaced_source_loads() {
    crate::test_support::gtk_test(
        "ui::preview::tests::find::pathological_code_previews_search_safely_and_cancel_replaced_source_loads",
        || {
            let source = SourcePreviewView::new();
            let window = gtk::Window::new();
            let normal = "let value = 1;\n".repeat(400);
            source.show(&entry("old.rs"), "text/x-rust", &normal, false);
            let old = source
                .view
                .borrow()
                .as_ref()
                .expect("active source preview")
                .buffer();
            assert!(source.fill.borrow().is_some());
            for (name, mime) in [
                ("large.json", "application/json"),
                ("large.xml", "application/xml"),
                ("large.js", "text/javascript"),
                ("large.txt", "text/plain"),
            ] {
                let text = format!("{} unique-needle", "x".repeat(1024 * 1024));
                let (view, _) = source.show(&entry(name), mime, &text, true);
                window.set_child(Some(&view));
                window.present();
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                assert!(source.fill.borrow().is_none());
                assert_eq!(old.char_count(), 0);
                let state = source
                    .virtual_state
                    .borrow()
                    .as_ref()
                    .expect("active source preview")
                    .clone();
                assert_eq!(state.find_matches("unique-needle").len(), 1, "{name}");
            }
            source.show(&entry("new.rs"), "text/x-rust", "let latest = 7;", false);
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            let buffer = source
                .view
                .borrow()
                .as_ref()
                .expect("active source preview")
                .buffer();
            assert_eq!(
                buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
                "let latest = 7;"
            );
            source.cancel();
            assert_eq!(buffer.char_count(), 0);
            window.destroy();
        },
    );
}

#[test]
fn source_find_tracks_incremental_text_and_resets_for_another_file() {
    crate::test_support::gtk_test(
        "ui::preview::tests::find::source_find_tracks_incremental_text_and_resets_for_another_file",
        || {
            let provider = Rc::new(Provider::default());
            let preview = PreviewDrawer::new(provider.clone(), false);
            preview.show(entry("source.rs"), None);
            let content = format!("{}// tail ÉCLAIR\n", "// padding line\n".repeat(400));
            {
                let pending = provider.0.borrow();
                let pending = &pending[0];
                (pending.emit)(PreviewEvent::Ready(Preview {
                    request_id: pending.request.id,
                    entry: pending.request.entry.clone(),
                    content_type: "text/x-rust".into(),
                    content: PreviewContent::Text {
                        content,
                        truncated: false,
                    },
                }));
            }
            assert!(
                preview
                    .state
                    .source_preview
                    .virtual_state
                    .borrow()
                    .is_none()
            );
            preview.state.find_button.emit_clicked();
            preview.state.find.entry.set_text("éclair");
            let buffer = preview
                .state
                .source_preview
                .view
                .borrow()
                .as_ref()
                .expect("source preview view")
                .buffer();
            assert!(buffer.selection_bounds().is_none());
            while glib::MainContext::default().iteration(false) {}
            let (start, end) = buffer
                .selection_bounds()
                .expect("find selects the later-loaded match");
            assert_eq!(buffer.text(&start, &end, true), "ÉCLAIR");
            preview.show(entry("other.txt"), None);
            assert!(!preview.state.find.widget.is_visible());
            assert!(preview.state.find.entry.text().is_empty());
            preview.close();
        },
    );
}
