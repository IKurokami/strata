// SPDX-License-Identifier: MIT

use super::*;

#[derive(Default)]
struct Provider;

impl PreviewProvider for Provider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        emit(PreviewEvent::Ready(Preview {
            request_id: request.id,
            entry: request.entry,
            content_type: "text/plain".into(),
            content: PreviewContent::Text {
                content: "ordinary **text**".into(),
                truncated: false,
            },
        }));
        LoadHandle::new(|| {})
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
            let drawer = PreviewDrawer::new(Rc::new(Provider), false);
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
            assert_eq!(
                std::fs::read_to_string(path).expect("unchanged file"),
                "ordinary **text**"
            );
            drawer.state.reset_content(None);
            assert!(!drawer.state.discord_edit.is_visible());
        },
    );
}
