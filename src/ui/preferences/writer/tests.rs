// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn bursts_persist_the_latest_snapshot_and_flush_waits_for_the_atomic_write() {
    let root = tempfile::tempdir().expect("settings fixture");
    let path = root.path().join("nested/settings.toml");
    let (sender, receiver) = mpsc::sync_channel(1);
    let pending = Arc::new(Mutex::new(None));
    let writer = Writer {
        sender,
        pending: pending.clone(),
    };
    let first = writer
        .save(1, path.clone(), Preferences::default())
        .expect("queue first");
    let expected = Preferences {
        preview_volume: 0.37,
        folder_peeking: true,
        ..Preferences::default()
    };
    let last = writer
        .save(2, path.clone(), expected.clone())
        .expect("queue latest");
    let worker = thread::spawn(move || run(receiver, pending));
    let completion = writer.flush().expect("flush").expect("completion");
    assert_eq!(completion.revision, 2);
    assert!(completion.result.is_ok());
    let saved: Preferences =
        toml::from_str(&fs::read_to_string(&path).expect("persisted settings")).expect("settings");
    assert_eq!(saved, expected);
    assert!(futures_lite::future::block_on(first).is_err());
    assert_eq!(
        futures_lite::future::block_on(last)
            .expect("latest completion")
            .revision,
        2
    );
    drop(writer);
    worker.join().expect("writer stopped");
}

#[test]
fn a_stopped_worker_reports_a_localizable_connection_error() {
    let root = tempfile::tempdir().expect("settings fixture");
    let path = root.path().join("settings.toml");
    let (sender, receiver) = mpsc::sync_channel(1);
    drop(receiver);
    let writer = Writer {
        sender,
        pending: Arc::new(Mutex::new(None)),
    };
    let error = writer
        .save(1, path.clone(), Preferences::default())
        .expect_err("stopped writer");
    assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
    assert_eq!(
        writer.flush().err().expect("stopped flush").kind(),
        io::ErrorKind::ConnectionAborted
    );
    assert!(
        !path.exists(),
        "a stopped worker must never fall back to synchronous I/O"
    );
}

#[test]
fn a_failed_write_can_be_retried_without_a_synchronous_fallback() {
    let root = tempfile::tempdir().expect("settings fixture");
    let path = root.path().join("settings.toml");
    fs::create_dir(&path).expect("block destination");
    let writer = Writer::new().expect("writer");
    let _first = writer
        .save(1, path.clone(), Preferences::default())
        .expect("queue first");
    assert!(
        writer
            .flush()
            .expect("flush failure")
            .expect("completion")
            .result
            .is_err()
    );
    fs::remove_dir(&path).expect("unblock fixture");
    let _retry = writer
        .save(2, path.clone(), Preferences::default())
        .expect("retry");
    assert!(
        writer
            .flush()
            .expect("flush retry")
            .expect("completion")
            .result
            .is_ok()
    );
    assert!(path.is_file());
}
