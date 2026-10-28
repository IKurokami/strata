// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn newest_request_mailbox_discards_earlier_keystrokes_without_spawning_workers() {
    let lane = Lane::new();
    let stop = AtomicBool::new(false);
    for generation in 1..=30 {
        lane.post(Arc::new(Request {
            generation,
            configuration: 1,
            query: format!("query {generation}"),
            roots: Vec::new(),
            kind: result::SearchKind::Any,
            submitted: Instant::now(),
        }));
    }
    assert_eq!(lane.next(&stop).expect("newest").generation, 30);
    stop.store(true, Ordering::Release);
    assert!(lane.next(&stop).is_none());
}

#[test]
fn locator_round_trip_preserves_unicode_page_and_media_units() {
    use result::ContentLocator;
    for locator in [
        ContentLocator::Pdf {
            page_0: 4,
            start_char: 12,
            end_char: 20,
            anchor: "hóa đơn 😀".into(),
        },
        ContentLocator::Audio {
            start_us: 25_000_000,
            end_us: 55_000_000,
        },
        ContentLocator::Sheet {
            name: "Doanh thu".into(),
            row_1: 42,
            col_1: 3,
            anchor: "giá".into(),
        },
    ] {
        let restored: ContentLocator =
            serde_json::from_str(&serde_json::to_string(&locator).expect("serialize"))
                .expect("deserialize");
        assert_eq!(restored, locator);
    }
    assert_eq!(
        ContentLocator::Video {
            start_us: 10_000_000,
            end_us: 20_000_000,
            soundtrack: false
        }
        .time_range(),
        Some((10_000_000, 20_000_000))
    );
}

#[test]
fn semantic_results_are_never_overwritten_by_a_late_keyword_phase_or_old_query() {
    use result::{SearchPhase, SearchResponse, accepts_phase};
    let mut response = SearchResponse {
        generation: 2,
        configuration: 1,
        phase: SearchPhase::Keyword,
        hits: Vec::new(),
    };
    assert!(accepts_phase(2, None, &response));
    assert!(!accepts_phase(2, Some(SearchPhase::Semantic), &response));
    response.phase = SearchPhase::Semantic;
    assert!(accepts_phase(2, Some(SearchPhase::Keyword), &response));
    assert!(!accepts_phase(3, None, &response));
}
