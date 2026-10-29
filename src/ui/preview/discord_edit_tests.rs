// SPDX-License-Identifier: MIT

use super::*;

struct Provider(RefCell<String>);
impl PreviewProvider for Provider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        emit(PreviewEvent::Ready(Preview {
            request_id: request.id,
            entry: request.entry,
            content_type: "text/plain".into(),
            content: PreviewContent::Text {
                content: self.0.borrow().clone(),
                truncated: false,
            },
        }));
        LoadHandle::new(|| {})
    }
}

fn wait(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "inline preview timed out"
        );
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn ordinary_text_preview_stays_read_only_and_hides_editing_on_reset() {
    crate::test_support::gtk_test(
        "ui::preview::discord_edit_tests::ordinary_text_preview_stays_read_only_and_hides_editing_on_reset",
        || {
            let folder = tempfile::tempdir().expect("fixture");
            let path = folder.path().join("plain.txt");
            std::fs::write(&path, "ordinary **text**").expect("source");
            let drawer = PreviewDrawer::new(
                Rc::new(Provider(RefCell::new("ordinary **text**".into()))),
                false,
            );
            let entry =
                crate::test_support::operations::entry(crate::model::Location::local(path.clone()));
            drawer.show(entry, None);
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            let source = drawer
                .state
                .source_preview
                .view
                .borrow()
                .clone()
                .expect("source preview");
            assert!(!source.is_editable());
            let buffer = source.buffer();
            assert_eq!(
                buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
                "ordinary **text**"
            );
            assert!(drawer.state.discord_edit.is_visible());
            assert!(drawer.state.discord_editor.borrow().is_none());
            assert_eq!(
                std::fs::read_to_string(path).expect("unchanged"),
                "ordinary **text**"
            );
            drawer.state.reset_content(None);
            assert!(!drawer.state.discord_edit.is_visible());
        },
    );
}

#[test]
fn fenced_ansi_renders_immediately_and_inline_drafts_survive_navigation() {
    crate::test_support::gtk_test(
        "ui::preview::discord_edit_tests::fenced_ansi_renders_immediately_and_inline_drafts_survive_navigation",
        || {
            let folder = tempfile::tempdir().expect("fixture");
            let path = folder.path().join("ansi.txt");
            let other = folder.path().join("other.txt");
            let original = "```ansi\n\x1b[1;36mảnh Việt\x1b[0m\n```";
            std::fs::write(&path, original).expect("source");
            std::fs::write(&other, "other").expect("other");
            let provider = Rc::new(Provider(RefCell::new(original.into())));
            let drawer = PreviewDrawer::new(provider.clone(), false);
            let window = gtk::Window::builder()
                .default_width(900)
                .default_height(600)
                .child(&drawer.widget())
                .build();
            window.present();
            let entry =
                crate::test_support::operations::entry(crate::model::Location::local(path.clone()));
            drawer.show(entry.clone(), None);
            let editor = drawer
                .state
                .discord_editor
                .borrow()
                .clone()
                .expect("rendered ANSI");
            let view = editor.text_view();
            assert!(!view.is_editable());
            let buffer = view.buffer();
            assert_eq!(
                buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
                "ảnh Việt"
            );
            assert!(
                buffer
                    .start_iter()
                    .tags()
                    .iter()
                    .any(|tag| tag.weight() == 700 && tag.foreground_rgba().is_some()),
                "ANSI styles are rendered without entering edit mode"
            );
            drawer.state.find_button.emit_clicked();
            drawer.state.find.entry.set_text("Việt");
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            let (start, end) = buffer.selection_bounds().expect("searchable rendered text");
            assert_eq!(buffer.text(&start, &end, true), "Việt");
            drawer.state.discord_edit.emit_clicked();
            wait(|| editor.text_view().is_editable());
            assert_eq!(
                editor.text_view(),
                view,
                "editing keeps the rendered preview surface"
            );
            buffer.insert_at_cursor(" mới");
            assert!(editor.has_draft());
            provider.0.replace("other".into());
            drawer.show(
                crate::test_support::operations::entry(crate::model::Location::local(other)),
                None,
            );
            assert!(drawer.state.discord_drafts.borrow().contains_key(&path));
            provider.0.replace(original.into());
            drawer.show(entry, None);
            let restored = drawer
                .state
                .discord_editor
                .borrow()
                .clone()
                .expect("restored draft");
            assert!(Rc::ptr_eq(&editor, &restored));
            assert!(
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .contains("mới")
            );
            assert_eq!(
                std::fs::read_to_string(path).expect("original unchanged"),
                original
            );
            window.close();
        },
    );
}
