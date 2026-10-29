use super::*;
use crate::services::disk_usage::Child;

fn snapshot(bytes: u64, resumable: bool) -> Snapshot {
    Snapshot {
        root: "fixture".into(),
        children: vec![Child {
            path: "fixture/item".into(),
            bytes,
            files: 1,
            directory: true,
        }],
        total: bytes,
        files: 1,
        other_bytes: 0,
        other_count: 0,
        coverage: if resumable {
            Coverage::Partial
        } else {
            Coverage::Exact
        },
        generation: 1,
        identity: None,
        resumable,
    }
}

#[test]
fn scan_progress_preserves_usable_results_until_a_terminal_publication() {
    let first = snapshot(10, true);
    assert!(present(None, &first));
    for bytes in [20, 30, 100] {
        assert!(!present(Some(&first), &snapshot(bytes, true)));
    }
    let complete = snapshot(100, false);
    assert!(present(Some(&first), &complete));
    assert!(content_changed(Some(&first), &complete));
}

#[test]
fn empty_initial_publications_do_not_hide_the_first_useful_results() {
    let empty = snapshot(0, true);
    let first = snapshot(10, true);
    assert!(present(Some(&empty), &first));
    let mut other = empty.clone();
    other.other_bytes = 50;
    assert!(present(Some(&empty), &other));
    assert!(!present(Some(&other), &first));
}

#[test]
fn cached_results_survive_stale_refresh_and_accept_partial_or_unavailable_completion() {
    let cached = snapshot(100, false);
    let mut incoming = snapshot(0, true);
    incoming.generation = 2;
    incoming.coverage = Coverage::Stale;
    assert!(!present(Some(&cached), &incoming));
    incoming.resumable = false;
    incoming.coverage = Coverage::Partial;
    assert!(present(Some(&cached), &incoming));
    incoming.coverage = Coverage::Unavailable;
    assert!(present(Some(&cached), &incoming));
}

#[test]
fn duplicate_and_progress_metadata_only_results_do_not_rebuild_content() {
    let displayed = snapshot(100, false);
    let mut incoming = displayed.clone();
    incoming.generation += 1;
    incoming.coverage = Coverage::Partial;
    incoming.files += 1;
    assert!(present(Some(&displayed), &incoming));
    assert!(!content_changed(Some(&displayed), &incoming));
    incoming.children[0].path = "fixture/replacement".into();
    assert!(content_changed(Some(&displayed), &incoming));
}
