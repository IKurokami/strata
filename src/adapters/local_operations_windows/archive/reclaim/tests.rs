// SPDX-License-Identifier: MIT

use super::*;
use std::{fs, io::Write, path::PathBuf};

fn archive(root: &Path, entries: &[(&str, &[u8])], password: Option<&str>) -> PathBuf {
    let path = root.join("sample.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&path).expect("archive"));
    for (name, bytes) in entries {
        let mut options = zip::write::SimpleFileOptions::default();
        if let Some(password) = password {
            options = options.with_aes_encryption(zip::AesMode::Aes256, password);
        }
        writer.start_file(*name, options).expect("entry");
        writer.write_all(bytes).expect("contents");
    }
    writer.finish().expect("finish archive");
    path
}

fn run(path: &Path, password: Option<&str>) -> Result<Option<String>, ArchiveError> {
    extract(
        path,
        password,
        &Arc::new(AtomicUsize::new(0)),
        &AtomicUsize::new(0),
        &AtomicBool::new(false),
    )
}

#[test]
fn reverse_extraction_consumes_archive_only_after_publishing_contents() {
    let root = tempfile::tempdir().expect("fixture");
    let path = archive(
        root.path(),
        &[("first.txt", b"first"), ("nested/last.txt", b"last")],
        None,
    );
    assert_eq!(run(&path, None).expect("extract"), Some("sample".into()));
    assert_eq!(
        fs::read(root.path().join("sample/first.txt")).expect("first"),
        b"first"
    );
    assert_eq!(
        fs::read(root.path().join("sample/nested/last.txt")).expect("last"),
        b"last"
    );
    assert!(!path.exists());
    assert!(!path.with_extension("zip.hatchzip").exists());
}

#[test]
fn collision_and_wrong_password_preserve_original_archive_and_existing_files() {
    for password in [None, Some("correct")] {
        let root = tempfile::tempdir().expect("fixture");
        let path = archive(root.path(), &[("notes.txt", b"new")], password);
        let original = fs::read(&path).expect("archive bytes");
        if password.is_none() {
            fs::create_dir(root.path().join("sample")).expect("folder");
            fs::write(root.path().join("sample/notes.txt"), b"existing").expect("existing");
        }
        let error = run(&path, password.map(|_| "wrong")).expect_err("refuse unsafe start");
        assert_eq!(fs::read(&path).expect("archive unchanged"), original);
        if password.is_some() {
            assert_eq!(
                error.password_failure(),
                Some(crate::services::PasswordFailure::Incorrect)
            );
            assert!(run(&path, Some("correct")).is_ok());
        } else {
            assert_eq!(
                fs::read(root.path().join("sample/notes.txt")).expect("existing"),
                b"existing"
            );
        }
    }
}

#[test]
fn paused_journal_resumes_in_the_same_output_folder() {
    let root = tempfile::tempdir().expect("fixture");
    let names: Vec<_> = (0..513).map(|index| format!("entry-{index}.txt")).collect();
    let entries: Vec<_> = names
        .iter()
        .map(|name| (name.as_str(), b"recover me".as_slice()))
        .collect();
    let path = archive(root.path(), &entries, None);
    let original_length = fs::metadata(&path).expect("archive").len();
    let (primary, paths) = session::locate(&path).expect("paths");
    let State::Fresh(job, output) = session::open(
        &primary,
        &paths,
        None,
        true,
        None,
        &Arc::new(Reads::default()),
    )
    .expect("open") else {
        panic!("expected fresh archive");
    };
    let journal = session::begin(&job, &paths, output, false).expect("begin");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_after_commit = stop.clone();
    let outcome = Run {
        job,
        journal,
        paths,
        stop: &stop,
        keep_free: KEEP_FREE,
        progress: Progress {
            on_bytes: Box::new(|_| {}),
            on_commit: Box::new(move |_| stop_after_commit.store(true, Ordering::Relaxed)),
        },
    }
    .execute()
    .expect("pause");
    assert!(matches!(outcome, Outcome::Paused));
    let journal_path = path.with_extension("zip.hatchzip");
    assert!(journal_path.exists());
    assert!(fs::metadata(&path).expect("remaining archive").len() < original_length);
    run(&journal_path, None).expect("resume through journal");
    for name in names {
        assert_eq!(
            fs::read(root.path().join("sample").join(name)).expect("contents"),
            b"recover me"
        );
    }
    assert!(!path.exists());
    assert!(!journal_path.exists());
}

#[test]
fn cancel_before_start_preserves_archive_without_creating_recovery_files() {
    let root = tempfile::tempdir().expect("fixture");
    let path = archive(root.path(), &[("notes.txt", b"keep")], None);
    let original = fs::read(&path).expect("archive");
    assert_eq!(
        extract(
            &path,
            None,
            &Arc::new(AtomicUsize::new(0)),
            &AtomicUsize::new(0),
            &AtomicBool::new(true)
        ),
        Err(ArchiveError::Cancelled)
    );
    assert_eq!(fs::read(&path).expect("archive"), original);
    assert!(!path.with_extension("zip.hatchzip").exists());
}

#[test]
fn recovery_cannot_redirect_truncation_or_cleanup_to_another_path() {
    for redirect_archive in [false, true] {
        let root = tempfile::tempdir().expect("fixture");
        let path = archive(root.path(), &[("notes.txt", b"new")], None);
        let original = fs::read(&path).expect("archive");
        let victim = root.path().join("unrelated.txt");
        fs::write(&victim, b"keep unrelated data").expect("unrelated file");
        let (primary, paths) = session::locate(&path).expect("paths");
        let State::Fresh(job, output) = session::open(
            &primary,
            &paths,
            None,
            true,
            None,
            &Arc::new(Reads::default()),
        )
        .expect("open") else {
            panic!("fresh archive");
        };
        let mut journal = session::begin(&job, &paths, output, false).expect("begin");
        drop(job);
        if redirect_archive {
            journal.parts[0].0 = victim.clone();
        } else {
            journal.output = root.path().to_path_buf();
        }
        journal.save(&paths).expect("modified recovery journal");
        assert!(run(&path, None).is_err());
        assert_eq!(
            fs::read(&victim).expect("unrelated file"),
            b"keep unrelated data"
        );
        assert_eq!(fs::read(&path).expect("archive"), original);
    }
}

#[test]
fn tar_and_sevenz_reclaim_space_without_changing_extracted_bytes() {
    for extension in ["tar", "7z"] {
        let root = tempfile::tempdir().expect("fixture");
        let source = root.path().join("source.txt");
        fs::write(&source, b"archive codec round trip").expect("source");
        let path = root.path().join(format!("sample.{extension}"));
        if extension == "tar" {
            let mut builder = tar::Builder::new(fs::File::create(&path).expect("tar"));
            builder
                .append_path_with_name(&source, "source.txt")
                .expect("tar entry");
            builder.finish().expect("finish tar");
        } else {
            let mut writer = sevenz_rust2::ArchiveWriter::new(fs::File::create(&path).expect("7z"))
                .expect("7z writer");
            writer
                .push_archive_entry(
                    sevenz_rust2::ArchiveEntry::new_file("source.txt"),
                    Some(fs::File::open(&source).expect("source")),
                )
                .expect("7z entry");
            writer.finish().expect("finish 7z");
        }
        run(&path, None).expect("space-saving extraction");
        assert_eq!(
            fs::read(root.path().join("sample/source.txt")).expect("output"),
            b"archive codec round trip"
        );
        assert_eq!(
            fs::read(&source).expect("source untouched"),
            b"archive codec round trip"
        );
        assert!(!path.exists());
    }
}
