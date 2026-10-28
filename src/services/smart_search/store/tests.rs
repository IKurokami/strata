// SPDX-License-Identifier: MIT

use super::*;
use std::collections::HashSet;

fn segment(text: &str) -> ExtractedSegment {
    ExtractedSegment {
        locator: ContentLocator::Text {
            chunk: 0,
            start_char: 0,
            end_char: text.chars().count() as u32,
            anchor: text.into(),
        },
        text: text.into(),
        modality: "text".into(),
    }
}

#[test]
fn fts_persists_unicode_resumable_segments_and_replaces_changed_revisions() {
    let directory = tempfile::tempdir().expect("fixture");
    let database = directory.path().join("index.sqlite");
    let path = directory.path().join("notes.txt");
    let revision = FileRevision {
        size: 8,
        modified_ns: 10,
    };
    let mut connection = connect(&database, true).expect("open");
    append(
        &mut connection,
        directory.path(),
        &path,
        &revision,
        &[segment("hóa đơn tiếng Việt")],
        &[None],
        None,
        Some(25600),
        false,
    )
    .expect("first batch");
    assert_eq!(
        cursor(&connection, &path, &revision).expect("cursor"),
        25600
    );
    assert!(!current(&connection, &path, &revision, None).expect("partial"));
    append(
        &mut connection,
        directory.path(),
        &path,
        &revision,
        &[segment("second section")],
        &[None],
        None,
        None,
        true,
    )
    .expect("second batch");
    drop(connection);
    let mut connection = connect(&database, true).expect("reopen");
    let roots = [directory.path().to_owned()];
    assert_eq!(
        keyword(
            &connection,
            "hoa don",
            &roots,
            false,
            super::super::result::SearchKind::Any
        )
        .expect("Unicode FTS")
        .len(),
        1
    );
    assert_eq!(
        keyword(
            &connection,
            "sec",
            &roots,
            false,
            super::super::result::SearchKind::Any
        )
        .expect("prefix FTS")
        .len(),
        1
    );
    assert!(current(&connection, &path, &revision, None).expect("complete"));
    let changed = FileRevision {
        size: 9,
        modified_ns: 11,
    };
    append(
        &mut connection,
        directory.path(),
        &path,
        &changed,
        &[segment("replacement")],
        &[None],
        None,
        None,
        false,
    )
    .expect("replace");
    assert!(
        keyword(
            &connection,
            "hoa",
            &roots,
            false,
            super::super::result::SearchKind::Any
        )
        .expect("stale text")
        .is_empty()
    );
    assert_eq!(
        keyword(
            &connection,
            "replacement",
            &roots,
            false,
            super::super::result::SearchKind::Any
        )
        .expect("replacement")
        .len(),
        1
    );
}

#[test]
fn offline_scan_preserves_entries_and_completed_scan_removes_only_index_records() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("keep.txt");
    std::fs::write(&path, "untouched").expect("source");
    let mut connection = connect(&directory.path().join("index.sqlite"), true).expect("open");
    let root = directory.path();
    let roots = [root.to_owned()];
    append(
        &mut connection,
        root,
        &path,
        &FileRevision {
            size: 9,
            modified_ns: 1,
        },
        &[segment("invoice")],
        &[None],
        None,
        None,
        false,
    )
    .expect("index");
    reconcile(&connection, &roots, &HashSet::new(), &[]).expect("offline reconciliation");
    assert_eq!(
        keyword(
            &connection,
            "invoice",
            &roots,
            false,
            super::super::result::SearchKind::Any
        )
        .expect("retained")
        .len(),
        1
    );
    reconcile(&connection, &roots, &HashSet::new(), &roots).expect("complete reconciliation");
    assert!(
        keyword(
            &connection,
            "invoice",
            &roots,
            false,
            super::super::result::SearchKind::Any
        )
        .expect("removed from FTS")
        .is_empty()
    );
    assert_eq!(
        std::fs::read_to_string(path).expect("source remains"),
        "untouched"
    );
}

#[test]
fn incompatible_fingerprint_and_nonfinite_vectors_are_not_queried() {
    let directory = tempfile::tempdir().expect("fixture");
    let root = directory.path();
    let mut connection = connect(&root.join("index.sqlite"), true).expect("open");
    append(
        &mut connection,
        root,
        &root.join("a.txt"),
        &FileRevision {
            size: 1,
            modified_ns: 2,
        },
        &[segment("needle")],
        &[Some(vec![f32::NAN; 768])],
        Some("old"),
        None,
        false,
    )
    .expect("index");
    assert!(
        vectors(
            &connection,
            "new",
            &[root.to_owned()],
            false,
            super::super::result::SearchKind::Any
        )
        .expect("new fingerprint")
        .hits
        .is_empty()
    );
    assert!(
        vectors(
            &connection,
            "old",
            &[root.to_owned()],
            false,
            super::super::result::SearchKind::Any
        )
        .expect("invalid vector")
        .hits
        .is_empty()
    );
}

#[test]
fn schema_rejects_unknown_future_versions_without_resetting_data() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("index.sqlite");
    let connection = connect(&path, true).expect("open");
    connection
        .pragma_update(None, "user_version", 999)
        .expect("future version");
    drop(connection);
    assert!(connect(&path, true).is_err());
    assert!(path.exists());
}

#[test]
fn type_filter_is_applied_before_keyword_candidate_limit() {
    let directory = tempfile::tempdir().expect("fixture");
    let root = directory.path();
    let mut connection = connect(&root.join("index.sqlite"), true).expect("open");
    let revision = FileRevision {
        size: 1,
        modified_ns: 1,
    };
    for index in 0..1001 {
        append(
            &mut connection,
            root,
            &root.join(format!("{index}.txt")),
            &revision,
            &[segment("needle")],
            &[None],
            None,
            None,
            false,
        )
        .expect("text");
    }
    let image = root.join("last.png");
    append(
        &mut connection,
        root,
        &image,
        &revision,
        &[segment("needle")],
        &[None],
        None,
        None,
        false,
    )
    .expect("image");
    let hits = keyword(
        &connection,
        "needle",
        &[root.to_owned()],
        false,
        super::super::result::SearchKind::Images,
    )
    .expect("filtered search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].path, image);
}

#[test]
fn untagged_embedding_requires_backfill_and_renamed_file_survives_old_root_removal() {
    let directory = tempfile::tempdir().expect("fixture");
    let old = directory.path().join("old");
    let new = directory.path().join("new");
    std::fs::create_dir_all(&old).expect("old root");
    std::fs::create_dir_all(&new).expect("new root");
    let path = old.join("notes.txt");
    std::fs::write(&path, "invoice").expect("source");
    let revision = FileRevision::from_metadata(&std::fs::metadata(&path).expect("metadata"));
    let mut connection = connect(&directory.path().join("index.sqlite"), true).expect("open");
    append(
        &mut connection,
        &old,
        &path,
        &revision,
        &[segment("invoice")],
        &[Some(vec![0.1; 768])],
        None,
        None,
        false,
    )
    .expect("index");
    assert!(!current(&connection, &path, &revision, Some("current")).expect("backfill"));
    let moved = new.join("renamed.txt");
    std::fs::rename(&path, &moved).expect("rename");
    assert!(current(&connection, &moved, &revision, None).expect("identity reuse"));
    reconcile(
        &connection,
        std::slice::from_ref(&new),
        &HashSet::from([moved.clone()]),
        std::slice::from_ref(&new),
    )
    .expect("reconcile");
    assert_eq!(
        keyword(
            &connection,
            "invoice",
            &[new],
            false,
            super::super::result::SearchKind::Any
        )
        .expect("renamed search")[0]
            .path,
        moved
    );
}
