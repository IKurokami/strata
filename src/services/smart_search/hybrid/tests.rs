// SPDX-License-Identifier: MIT

use super::*;
use crate::services::smart_search::result::{ContentLocator, FileRevision};
use std::path::Path;

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

fn vector_index(candidates: &[(&str, &str, &str, f32)]) -> VectorIndex {
    let mut index = VectorIndex {
        hits: Vec::new(),
        values: Vec::new(),
    };
    for (position, &(path, modality, text, cosine)) in candidates.iter().enumerate() {
        let mut hit = fixture(path, text);
        hit.modality = modality.into();
        hit.segment_id = position as i64;
        index.hits.push(hit);
        let mut row = vec![half::f16::ZERO; 768];
        row[0] = half::f16::from_f32(cosine);
        row[1] = half::f16::from_f32((1.0 - cosine * cosine).sqrt());
        index.values.extend(row);
    }
    index
}

fn query_vector() -> Vec<f32> {
    let mut vector = vec![0.0; 768];
    vector[0] = 1.0;
    vector
}

#[test]
fn lexical_matching_uses_words_and_deduplicates_query_terms() {
    let hit = fixture("thanh.txt", "Vật liệu trang trí");
    assert_eq!(LexicalQuery::new("ảnh nhân").coverage(&hit), 0.0);
    assert_eq!(LexicalQuery::new("VẬT, vật, lieu!").coverage(&hit), 1.0);
}

#[test]
fn visual_intent_ranks_anonymous_image_above_text_with_incidental_query_words() {
    let index = vector_index(&[
        ("index.html", "text", "ảnh nhân vật anime", 0.68),
        ("bangthan.jpg", "image", "", 0.643),
        ("other.jpg", "image", "", 0.51),
    ]);
    let hits = rank(
        "ảnh nhân vật anime",
        &query_vector(),
        None,
        &index,
        Vec::new(),
        || false,
    );
    assert_eq!(hits[0].path, Path::new("bangthan.jpg"));
    assert_eq!(hits[0].reason, MatchReason::Semantic);
    assert!(!hits[0].weak);
    assert!(hits.iter().any(|hit| hit.path == Path::new("index.html")));
}

#[test]
fn capped_outlier_and_keyword_bonuses_preserve_a_better_semantic_match() {
    let index = vector_index(&[
        (
            "relevant.txt",
            "text",
            "A different wording of the intended subject",
            0.85,
        ),
        ("noise.rs", "code", "intended subject", 0.65),
    ]);
    let hits = rank(
        "intended subject",
        &query_vector(),
        None,
        &index,
        Vec::new(),
        || false,
    );
    assert_eq!(hits[0].path, Path::new("relevant.txt"));
}

#[test]
fn segment_duplicates_cannot_exhaust_the_file_result_budget() {
    let mut candidates = vec![("long.txt", "text", "", 0.8); 120];
    candidates.push(("bangthan.jpg", "image", "", 0.64));
    let index = vector_index(&candidates);
    let hits = rank("anime", &query_vector(), None, &index, Vec::new(), || false);
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().any(|hit| hit.path == Path::new("bangthan.jpg")));
}

#[test]
fn adding_duplicate_segments_cannot_inflate_a_files_modality_outlier_score() {
    let candidates = [
        ("relevant.txt", "text", "", 0.72),
        ("long.txt", "text", "", 0.64),
        ("background.txt", "text", "", 0.5),
    ];
    let baseline = rank(
        "subject",
        &query_vector(),
        None,
        &vector_index(&candidates),
        Vec::new(),
        || false,
    );
    let mut expanded = candidates.to_vec();
    expanded.extend(std::iter::repeat_n(candidates[1], 1000));
    let repeated = rank(
        "subject",
        &query_vector(),
        None,
        &vector_index(&expanded),
        Vec::new(),
        || false,
    );
    assert_eq!(
        baseline
            .iter()
            .map(|hit| (&hit.path, hit.score, hit.weak))
            .collect::<Vec<_>>(),
        repeated
            .iter()
            .map(|hit| (&hit.path, hit.score, hit.weak))
            .collect::<Vec<_>>()
    );
}

#[test]
fn media_preference_is_conservative_and_never_hides_exact_file_requests() {
    for query in [
        "ảnh hưởng của thời tiết",
        "photochemistry",
        "how to crop an image",
        "bangthan.jpg",
    ] {
        assert_eq!(requested_modality(query), None, "{query}");
    }
    for query in ["ảnh anime", "image of a cat", "画像"] {
        assert_eq!(requested_modality(query), Some("image"), "{query}");
    }
    let index = vector_index(&[
        ("video.mp4", "video", "", 0.9),
        ("video.txt", "text", "", 0.5),
    ]);
    let hits = rank(
        "video.txt",
        &query_vector(),
        None,
        &index,
        Vec::new(),
        || false,
    );
    assert_eq!(hits[0].path, Path::new("video.txt"));
}
