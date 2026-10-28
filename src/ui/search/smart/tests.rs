// SPDX-License-Identifier: MIT

use super::*;
use crate::services::smart_search::result::{
    ContentLocator, FileRevision, MatchReason, SearchHit,
};

fn hit(path: &str, segment: i64, score: i64, weak: bool) -> SearchHit {
    SearchHit {
        file_id: 1,
        segment_id: segment,
        path: path.into(),
        revision: FileRevision { size: 10, modified_ns: 5 },
        locator: ContentLocator::Audio {
            start_us: segment as u64 * 1_000_000,
            end_us: (segment as u64 + 5) * 1_000_000,
        },
        excerpt: String::new(),
        modality: "audio".into(),
        reason: MatchReason::Semantic,
        score,
        weak,
    }
}

#[test]
fn filename_results_remain_available_during_content_refresh() {
    crate::test_support::gtk_test(
        "ui::search::smart::tests::filename_results_remain_available_during_content_refresh",
        || {
            let panel = Panel::new(Rc::new(|_| {}));
            panel.query("invoice");
            let filename = SearchItem::for_test("/fixture/invoice.txt".into(), false);
            panel.names(vec![filename.clone()]);
            assert_eq!(panel.store.n_items(), 1);
            assert!(panel.drawer.borrow().is_none());
            assert!(panel.selected().is_none());
            assert_eq!(panel.activation_target().expect("default target").path, filename.path);
            panel.move_selection(1);
            assert_eq!(panel.selected().expect("selection").path, filename.path);
            panel.refresh_content();
            assert_eq!(panel.store.n_items(), 1);
            assert_eq!(panel.selected().expect("retained selection").path, filename.path);
            panel.cancel();
            assert!(!panel.preview_slot.is_visible());
        },
    );
}

#[test]
fn multiple_segments_share_one_row_and_keep_selected_file_across_updates() {
    crate::test_support::gtk_test(
        "ui::search::smart::tests::multiple_segments_share_one_row_and_keep_selected_file_across_updates",
        || {
            let panel = Panel::new(Rc::new(|_| {}));
            let path = "/fixture/clip.mp4";
            panel.names(vec![
                SearchItem::for_test(path.into(), false),
                SearchItem::for_test("/fixture/other.txt".into(), false),
                SearchItem::for_test(path.into(), false),
            ]);
            panel.hits.replace(vec![
                hit(path, 1, 5, true),
                hit(path, 2, 10, false),
                hit(path, 3, 8, false),
            ]);
            panel.render();
            assert_eq!(panel.store.n_items(), 2);
            assert_eq!(panel.rows.borrow()[0].path, std::path::PathBuf::from(path));
            assert_eq!(panel.rows.borrow()[0].content_hit.as_ref().unwrap().segment_id, 2);
            panel.move_selection(1);
            assert_eq!(panel.selected().expect("selection").path, path);
            panel.hits.replace(vec![hit(path, 4, 20, false)]);
            panel.render();
            assert_eq!(panel.store.n_items(), 2);
            assert_eq!(panel.selected().expect("stable selection").path, path);
            assert_eq!(panel.selected().unwrap().content_hit.unwrap().segment_id, 4);
            panel.cancel();
        },
    );
}

#[test]
fn only_distinct_files_are_shown_for_semantic_segments() {
    let rows = unique_rows(&[], &[
        hit("/files/video.mp4", 1, 9, false),
        hit("/files/video.mp4", 2, 20, false),
        hit("/files/audio.mp3", 3, 7, true),
    ]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].path, std::path::PathBuf::from("/files/video.mp4"));
    assert_eq!(rows[0].content_hit.as_ref().unwrap().segment_id, 2);
    assert_eq!(rows[1].path, std::path::PathBuf::from("/files/audio.mp3"));
}

#[test]
fn win32_verbatim_paths_are_stripped_only_for_display() {
    assert_eq!(
        display_search_path(Path::new(r"\\?\C:\Users\Public\file.txt")),
        r"C:\Users\Public\file.txt"
    );
    assert_eq!(
        display_search_path(Path::new(r"\\?\UNC\server\share\file.txt")),
        r"\\server\share\file.txt"
    );
    assert_eq!(
        display_search_path(Path::new("//?/C:/Users/Public/file.txt")),
        "C:/Users/Public/file.txt"
    );
    assert_eq!(display_search_path(Path::new("/home/user/file.txt")), "/home/user/file.txt");
}
