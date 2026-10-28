// SPDX-License-Identifier: MIT

use super::*;

fn measure(path: &Path, known: &HashMap<PathBuf, Snapshot>) -> Vec<Snapshot> {
    let mut scan = Scan::new(path.to_path_buf(), 1);
    while !scan.step(known) {}
    scan.snapshots(true)
}

#[test]
fn aggregates_nested_files_and_reuses_completed_subtrees() {
    let directory = tempfile::tempdir().unwrap();
    let child = directory.path().join("child");
    fs::create_dir(&child).unwrap();
    fs::write(child.join("first"), [1; 20]).unwrap();
    fs::write(directory.path().join("second"), [2; 7]).unwrap();
    let known: HashMap<_, _> = measure(&child, &HashMap::new())
        .into_iter()
        .map(|snapshot| (snapshot.root.clone(), snapshot))
        .collect();
    let snapshots = measure(directory.path(), &known);
    let root = snapshots
        .iter()
        .find(|snapshot| snapshot.root == directory.path())
        .unwrap();
    assert_eq!(
        (root.total, root.files, root.coverage),
        (27, 2, Coverage::Exact)
    );
    assert!(
        !snapshots.iter().any(|snapshot| snapshot.root == child),
        "cached subtree is not scanned or replaced with an empty snapshot"
    );
    fs::write(child.join("first"), [1; 25]).unwrap();
    let mut stale = known;
    stale.get_mut(&child).unwrap().coverage = Coverage::Stale;
    let updated = measure(directory.path(), &stale);
    assert_eq!(
        updated
            .iter()
            .find(|snapshot| snapshot.root == directory.path())
            .unwrap()
            .total,
        32
    );
}

#[test]
fn partial_publication_and_unavailable_roots_do_not_report_exact_zero() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("pending")).unwrap();
    fs::write(directory.path().join("pending/file"), [0; 13]).unwrap();
    let mut scan = Scan::new(directory.path().to_path_buf(), 9);
    assert!(!scan.step(&HashMap::new()));
    assert_eq!(scan.snapshots(false)[0].coverage, Coverage::Partial);
    while !scan.step(&HashMap::new()) {}
    assert_eq!(scan.snapshots(true)[0].total, 13);
    let missing = measure(&directory.path().join("not-present"), &HashMap::new());
    assert_eq!(missing[0].coverage, Coverage::Unavailable);
}

#[cfg(unix)]
#[test]
fn symbolic_links_never_follow_cycles_or_escape_the_scope() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("private"), [0; 33]).unwrap();
    fs::write(directory.path().join("local"), [0; 7]).unwrap();
    std::os::unix::fs::symlink(directory.path(), directory.path().join("cycle")).unwrap();
    std::os::unix::fs::symlink(outside.path(), directory.path().join("outside")).unwrap();
    let snapshots = measure(directory.path(), &HashMap::new());
    assert_eq!(snapshots[0].total, 7);
    assert_eq!(snapshots[0].coverage, Coverage::Partial);
    assert_eq!(snapshots[0].children.len(), 1);
}

#[test]
fn bounded_child_groups_preserve_all_known_bytes() {
    let directory = tempfile::tempdir().unwrap();
    for index in 0..605 {
        fs::write(directory.path().join(index.to_string()), [0; 2]).unwrap();
    }
    let mut scan = Scan::new(directory.path().to_path_buf(), 1);
    assert!(
        !scan.step(&HashMap::new()),
        "large directories publish before enumeration finishes"
    );
    let partial = scan.snapshots(false).remove(0);
    assert!(partial.files > 0 && partial.files < 605);
    assert_eq!(partial.coverage, Coverage::Partial);
    while !scan.step(&HashMap::new()) {}
    let snapshot = scan.snapshots(true).remove(0);
    assert_eq!(snapshot.children.len(), 300);
    assert_eq!(snapshot.other_count, 305);
    assert_eq!(snapshot.other_bytes, 610);
    assert_eq!(snapshot.total, 1210);
    assert_eq!(
        snapshot
            .children
            .iter()
            .map(|child| child.bytes)
            .sum::<u64>()
            + snapshot.other_bytes,
        snapshot.total
    );
}

#[test]
fn persistent_snapshots_roundtrip_and_corruption_keeps_scanning_available() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), [0; 9]).unwrap();
    let snapshot = measure(&source, &HashMap::new()).remove(0);
    let database = directory.path().join("usage.sqlite");
    let db = connect(&database).unwrap();
    db.execute(
        "INSERT INTO snapshots VALUES(?,?,0)",
        params![
            source.to_string_lossy(),
            serde_json::to_vec(&snapshot).unwrap()
        ],
    )
    .unwrap();
    drop(db);
    let db = connect(&database).unwrap();
    let bytes: Vec<u8> = db
        .query_row("SELECT data FROM snapshots", [], |row| row.get(0))
        .unwrap();
    let restored: Snapshot = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        (restored.total, restored.files, restored.coverage),
        (9, 1, Coverage::Exact)
    );
    let broken = directory.path().join("broken.sqlite");
    fs::write(&broken, b"not a database").unwrap();
    assert!(connect(&broken).is_err());
    assert_eq!(measure(&source, &HashMap::new())[0].total, 9);
}
