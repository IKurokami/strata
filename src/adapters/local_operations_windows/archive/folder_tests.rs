// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn named_extraction_preserves_existing_folders_and_archive_contents() {
    for archive in ["ảnh.v2.ZIP", "backup.tar.gz", "backup.tgz", "data.7z"] {
        let root = tempfile::tempdir().expect("archive extraction fixture");
        let stem = if archive.starts_with("backup") {
            "backup"
        } else if archive.starts_with("data") {
            "data"
        } else {
            "ảnh.v2"
        };
        let existing = root.path().join(stem);
        std::fs::create_dir(&existing).expect("archive extraction fixture");
        std::fs::write(existing.join("keep"), b"untouched").expect("archive extraction fixture");
        let name = extract_named_folder(root.path(), archive, &AtomicBool::new(false), |folder| {
            std::fs::create_dir(folder.join("inner")).expect("archive extraction fixture");
            std::fs::write(folder.join("inner/member.txt"), b"contents")
                .expect("archive extraction fixture");
            Ok(Some("inner".to_owned()))
        })
        .expect("archive extraction fixture")
        .expect("archive extraction fixture");
        assert_eq!(name, format!("{stem} (1)"));
        assert_eq!(
            std::fs::read(existing.join("keep")).expect("archive extraction fixture"),
            b"untouched"
        );
        assert_eq!(
            std::fs::read(root.path().join(&name).join("inner/member.txt"))
                .expect("archive extraction fixture"),
            b"contents"
        );
    }
}

#[test]
fn failed_named_extraction_cleans_only_its_empty_folder() {
    let root = tempfile::tempdir().expect("archive extraction fixture");
    for attempt in 0..2 {
        let error =
            extract_named_folder(root.path(), "secret.zip", &AtomicBool::new(false), |_| {
                Err(ArchiveError::PasswordRequired(PASSWORD_REQUIRED.to_owned()))
            })
            .expect_err("archive extraction must fail");
        assert!(
            matches!(error, ArchiveError::PasswordRequired(_)),
            "attempt {attempt}"
        );
        assert!(!root.path().join("secret").exists());
    }
    let result = extract_named_folder(
        root.path(),
        "secret.zip",
        &AtomicBool::new(false),
        |folder| {
            std::fs::write(folder.join("completed"), b"keep").expect("archive extraction fixture");
            Err(ArchiveError::Cancelled)
        },
    );
    assert!(matches!(result, Err(ArchiveError::Cancelled)));
    assert_eq!(
        std::fs::read(root.path().join("secret/completed")).expect("archive extraction fixture"),
        b"keep"
    );
}
