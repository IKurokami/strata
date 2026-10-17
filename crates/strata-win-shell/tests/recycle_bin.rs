#![cfg(windows)]

use std::{fs, io, path::PathBuf};

use strata_win_shell::{permanently_delete, recycle, restore};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("strata-win-shell-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch folder");
    dir
}

#[test]
fn recycled_files_leave_the_folder_and_restore_to_the_same_path() {
    let dir = scratch("roundtrip");
    let first = dir.join("first.txt");
    let nested = dir.join("folder");
    fs::write(&first, "one").unwrap();
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("inner.txt"), "two").unwrap();

    let items = recycle(&[first.clone(), nested.clone()]).expect("recycle batch");

    assert_eq!(items.len(), 2);
    assert!(!first.exists() && !nested.exists());
    assert!(items.iter().all(|item| item.recycled.exists()));
    assert!(items.iter().any(|item| item.original == first));

    for item in &items {
        restore(item).expect("restore");
    }
    assert_eq!(fs::read_to_string(&first).unwrap(), "one");
    assert_eq!(fs::read_to_string(nested.join("inner.txt")).unwrap(), "two");
    assert!(items.iter().all(|item| !item.recycled.exists()));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn restore_never_replaces_a_file_created_at_the_original_path() {
    let dir = scratch("conflict");
    let path = dir.join("doc.txt");
    fs::write(&path, "deleted").unwrap();
    let item = recycle(std::slice::from_ref(&path))
        .expect("recycle")
        .remove(0);
    fs::write(&path, "newer").unwrap();

    let error = restore(&item).expect_err("restore must refuse");

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&path).unwrap(), "newer");
    assert!(item.recycled.exists(), "the recycled copy must survive");
    // Leave nothing behind in the user's real Recycle Bin.
    fs::remove_file(&path).unwrap();
    restore(&item).expect("restore after clearing the conflict");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn relative_and_network_paths_are_rejected_without_deleting() {
    for path in ["relative.txt", r"\\server\share\file.txt"] {
        let error = recycle(&[PathBuf::from(path)]).expect_err("must reject");
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }
}

#[test]
fn restore_by_original_path_finds_and_restores_recycled_file() {
    let dir = scratch("restore-orig");
    let path = dir.join("unique-test.txt");
    fs::write(&path, "restorable content").unwrap();

    let _ = recycle(std::slice::from_ref(&path)).expect("recycle");
    assert!(!path.exists());

    let restored = strata_win_shell::restore_by_original_path(&path).expect("restore by path");
    assert_eq!(restored, path);
    assert!(path.exists());
    assert_eq!(fs::read_to_string(&path).unwrap(), "restorable content");

    fs::remove_file(&path).unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn permanent_delete_removes_an_explicitly_confirmed_file() {
    let dir = scratch("permanent");
    let path = dir.join("permanent.txt");
    fs::write(&path, "gone").unwrap();

    permanently_delete(std::slice::from_ref(&path)).expect("permanent delete");

    assert!(!path.exists());
    fs::remove_dir_all(dir).unwrap();
}
