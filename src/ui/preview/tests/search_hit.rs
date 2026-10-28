// SPDX-License-Identifier: MIT

use super::*;
use crate::services::smart_search::result::{ContentLocator, FileRevision, MatchReason, SearchHit};

#[test]
fn smart_search_pdf_locator_is_carried_to_the_provider_and_reloads_another_page() {
    crate::test_support::gtk_test(
        "ui::preview::tests::search_hit::smart_search_pdf_locator_is_carried_to_the_provider_and_reloads_another_page",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            let make_hit = |page| {
                Arc::new(SearchHit {
                    file_id: 1,
                    segment_id: page as i64,
                    path: "/fixture/document.pdf".into(),
                    revision: FileRevision {
                        size: 100,
                        modified_ns: 5,
                    },
                    locator: ContentLocator::Pdf {
                        page_0: page,
                        start_char: 4,
                        end_char: 10,
                        anchor: "hóa đơn".into(),
                    },
                    excerpt: "hóa đơn".into(),
                    modality: "text".into(),
                    reason: MatchReason::Keyword,
                    score: 1,
                    weak: false,
                })
            };
            drawer.show_hit(make_hit(4), None);
            {
                let pending = provider.0.borrow();
                assert_eq!(pending.len(), 1);
                assert_eq!(pending[0].request.pdf_page, 4);
                assert!(pending[0].request.search_hit.is_some());
            }
            drawer.show_hit(make_hit(7), None);
            {
                let pending = provider.0.borrow();
                assert_eq!(pending.len(), 2);
                assert_eq!(pending[1].request.pdf_page, 7);
            }
            drawer.show(entry("normal.txt"), None);
            assert!(
                provider
                    .0
                    .borrow()
                    .last()
                    .expect("normal preview")
                    .request
                    .search_hit
                    .is_none()
            );
        },
    );
}

#[test]
fn flashraw_develop_request_retains_the_embedded_preview_until_replacement() {
    crate::test_support::gtk_test(
        "ui::preview::tests::search_hit::flashraw_develop_request_retains_the_embedded_preview_until_replacement",
        || {
            crate::sandbox::pixels::set_fast_raw(true);
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.show(entry("fixture.dng"), None);
            let id = provider.0.borrow()[0].request.id;
            let pixels = vec![40; 4 * 4 * 4];
            (provider.0.borrow()[0].emit)(PreviewEvent::Ready(Preview {
                request_id: id,
                entry: entry("fixture.dng"),
                content_type: "image/x-adobe-dng".into(),
                content: PreviewContent::RawImage {
                    width: 4,
                    height: 4,
                    pixels,
                },
            }));
            let before = drawer
                .state
                .content
                .first_child()
                .expect("embedded preview");
            drawer.state.develop.emit_clicked();
            assert_eq!(drawer.state.content.first_child(), Some(before));
            assert!(
                provider
                    .0
                    .borrow()
                    .last()
                    .expect("development request")
                    .request
                    .raw_develop
            );
        },
    );
}

#[test]
fn smart_search_pdf_hit_selects_the_matched_unicode_text_from_the_glyph_layer() {
    crate::test_support::gtk_test(
        "ui::preview::tests::search_hit::smart_search_pdf_hit_selects_the_matched_unicode_text_from_the_glyph_layer",
        || {
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            let text = "😀 hóa đơn khác";
            drawer.show_hit(
                Arc::new(SearchHit {
                    file_id: 1,
                    segment_id: 1,
                    path: "/fixture/paper.pdf".into(),
                    revision: FileRevision {
                        size: 100,
                        modified_ns: 1,
                    },
                    locator: ContentLocator::Pdf {
                        page_0: 3,
                        start_char: 2,
                        end_char: 9,
                        anchor: "hóa đơn".into(),
                    },
                    excerpt: "hóa đơn".into(),
                    modality: "text".into(),
                    reason: MatchReason::Keyword,
                    score: 1,
                    weak: false,
                }),
                None,
            );
            {
                let pending = provider.0.borrow();
                let pending = &pending[0];
                (pending.emit)(PreviewEvent::Ready(Preview {
                    request_id: pending.request.id,
                    entry: pending.request.entry.clone(),
                    content_type: "application/pdf".into(),
                    content: PreviewContent::Pdf {
                        png: Vec::new(),
                        page: 3,
                        pages: 5,
                        text_layer: Some(Arc::new(PdfTextLayer {
                            width: 200.0,
                            height: 20.0,
                            text: text.into(),
                            glyphs: text
                                .chars()
                                .enumerate()
                                .map(|(index, _)| {
                                    [index as f32 * 10.0, 0.0, index as f32 * 10.0 + 9.0, 12.0]
                                })
                                .collect(),
                        })),
                    },
                }));
            }
            let mut child = drawer.state.content.first_child();
            let mut controller = None;
            while let Some(widget) = child {
                if widget.has_css_class("preview-pdf-scroll") {
                    let controllers = widget.observe_controllers();
                    controller = (0..controllers.n_items()).find_map(|index| {
                        controllers
                            .item(index)
                            .and_downcast::<gtk::EventControllerKey>()
                    });
                    break;
                }
                child = widget.next_sibling();
            }
            let clipboard = drawer.widget().clipboard();
            clipboard.set_text("unchanged");
            assert!(controller.expect("PDF input").emit_by_name::<bool>(
                "key-pressed",
                &[
                    &gtk::gdk::Key::c,
                    &0u32,
                    &gtk::gdk::ModifierType::CONTROL_MASK
                ]
            ));
            let copied = clipboard
                .content()
                .and_then(|content| content.value(glib::Type::STRING).ok())
                .and_then(|value| value.get::<String>().ok())
                .expect("selected text");
            assert_eq!(copied, "hóa đơn");
        },
    );
}

#[test]
fn smart_search_media_hit_seeks_in_microseconds_and_user_input_cancels_delayed_reveal() {
    crate::test_support::gtk_test(
        "ui::preview::tests::search_hit::smart_search_media_hit_seeks_in_microseconds_and_user_input_cancels_delayed_reveal",
        || {
            crate::ui::media::use_test_streams(true);
            crate::ui::preferences::PreferenceManager::shared().set_preview_autoplay(false);
            for cancel in [false, true] {
                let provider = Rc::new(Provider::default());
                let drawer = PreviewDrawer::new(provider.clone(), false);
                drawer.show_hit(
                    Arc::new(SearchHit {
                        file_id: 1,
                        segment_id: 1,
                        path: "/fixture/clip.mp4".into(),
                        revision: FileRevision {
                            size: 100,
                            modified_ns: 1,
                        },
                        locator: ContentLocator::Video {
                            start_us: 25_500_000,
                            end_us: 30_000_000,
                            soundtrack: false,
                        },
                        excerpt: String::new(),
                        modality: "video".into(),
                        reason: MatchReason::Semantic,
                        score: 1,
                        weak: false,
                    }),
                    None,
                );
                super::video::ready(&provider, 0, "video/mp4");
                let stream = drawer.state.media.borrow().clone().expect("stream");
                stream.stream_prepared(true, true, true, 60_000_000);
                if cancel {
                    drawer.state.cancel_search_reveal();
                }
                let until = Instant::now() + Duration::from_millis(100);
                while Instant::now() < until {
                    while glib::MainContext::default().pending() {
                        glib::MainContext::default().iteration(false);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                assert_eq!(stream.timestamp(), if cancel { 0 } else { 25_500_000 });
                drawer.close();
            }
        },
    );
}
