// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn windows_folder_save_lock_serializes_writers_and_releases_on_close() {
    let directory = tempfile::tempdir().expect("folder settings directory");
    let path = directory.path().join("folder-views.toml");
    let first = lock_saves(&path, Duration::ZERO).expect("first writer lock");
    let error = lock_saves(&path, Duration::ZERO).expect_err("second writer must wait");
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    drop(first);
    let _next = lock_saves(&path, Duration::ZERO).expect("lock released after writer closes");
}
