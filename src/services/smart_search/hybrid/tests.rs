// SPDX-License-Identifier: MIT

use super::*;
use crate::services::smart_search::result::{ContentLocator, FileRevision};

pub(in crate::services::smart_search) fn fixture(path: &str, text: &str) -> SearchHit {
    SearchHit {
        file_id: 1,
        segment_id: 1,
        path: path.into(),
        revision: FileRevision {
            size: 10,
            modified_ns: 5,
        },
        locator: ContentLocator::Text {
            chunk: 0,
            start_char: 40,
            end_char: 100,
            anchor: String::new(),
        },
        excerpt: text.into(),
        modality: "text".into(),
        reason: MatchReason::Keyword,
        score: 0,
        weak: false,
    }
}

#[test]
fn keyword_locator_maps_vietnamese_and_non_ascii_offsets_back_to_original_text() {
    let mut hit = fixture("notes.txt", "😀 trước hóa đơn 2026 rồi");
    locate("hoa don", &mut hit);
    let ContentLocator::Text {
        start_char,
        end_char,
        anchor,
        ..
    } = hit.locator
    else {
        panic!("text locator")
    };
    assert_eq!(anchor, "hóa đơn");
    assert_eq!(start_char, 48);
    assert_eq!(end_char, 55);
}

#[test]
fn hybrid_keeps_exact_keyword_hits_and_separates_database_identities() {
    let first = fixture("invoice-123.txt", "invoice 123");
    let second = fixture("source.rs", "invoice 123");
    assert_ne!(first.identity(), second.identity());
    let index = VectorIndex {
        hits: vec![second.clone()],
        values: vec![half::f16::ZERO; 768],
    };
    let results = rank(
        "invoice-123.txt",
        &vec![0.0; 768],
        None,
        &index,
        vec![first.clone()],
        || false,
    );
    assert!(results.iter().any(|hit| hit.path == first.path));
    assert!(
        rank("invoice", &vec![0.0; 768], None, &index, Vec::new(), || {
            true
        })
        .is_empty()
    );
}
