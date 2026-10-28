// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn resumed_unicode_chunks_cover_the_document_without_gaps() {
    let text = "hóa đơn 😀\n".repeat(6000);
    let count = text.chars().count();
    let mut cursor = 0;
    let mut rebuilt = String::new();
    loop {
        let batch = text_chunks(&text, cursor, false, None);
        for segment in batch.segments {
            rebuilt.push_str(&segment.text);
        }
        if let Some(next) = batch.next {
            assert!(next > cursor);
            cursor = next;
        } else {
            break;
        }
    }
    assert_eq!(rebuilt, text);
    assert_eq!(rebuilt.chars().count(), count);
}

#[test]
fn code_chunks_keep_one_based_lines_and_pdf_uses_zero_based_pages() {
    let code = text_chunks("first\nsecond\n", 0, true, None);
    assert!(matches!(
        code.segments[0].locator,
        ContentLocator::Code {
            line_1: 1,
            end_line_1: 3,
            ..
        }
    ));
    let pdf = text_chunks("😀 hóa đơn", 0, false, Some(2));
    assert!(matches!(
        pdf.segments[0].locator,
        ContentLocator::Pdf {
            page_0: 2,
            start_char: 0,
            end_char: 9,
            ..
        }
    ));
    assert_eq!(
        cursor_parts(next_part(10, 25600).expect("cursor")),
        (10, 25600)
    );
    assert!(next_part(1024, 0).is_err());
}

#[test]
fn open_document_slides_keep_separate_locations_and_xml_entities() {
    let xml=br#"<root xmlns:draw="draw" xmlns:text="text"><draw:page><text:p>first &amp; next</text:p></draw:page><draw:page><text:p>second</text:p></draw:page></root>"#;
    assert_eq!(
        odp_pages(xml).expect("pages"),
        ["first & next\n", "second\n"]
    );
}

#[test]
fn binary_and_oversized_plain_text_are_rejected_before_indexing() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("file.txt");
    File::create(&path)
        .expect("fixture")
        .set_len(TEXT_LIMIT + 1)
        .expect("size");
    assert!(read(&path, 0).is_err());
    std::fs::write(&path, b"binary\0payload").expect("write");
    assert!(read(&path, 0).is_err());
    std::fs::write(&path, [0xff, 0xfe, b'A', 0, b'B', 0]).expect("UTF16");
    let extraction: Extraction =
        serde_json::from_slice(&read(&path, 0).expect("UTF16 extraction")).expect("JSON");
    assert_eq!(extraction.segments[0].text, "AB");
}
