use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU8, AtomicUsize, Ordering},
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "strata-merge-move-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&dir).expect("make fixture");
        Self(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("clean fixture");
    }
}

fn pending() -> AtomicU8 {
    AtomicU8::new(0)
}

#[test]
fn merge_combines_directories_and_keeps_existing_files() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.path("src/sub")).unwrap();
    fs::create_dir_all(fx.path("dst/sub")).unwrap();
    fs::write(fx.path("src/new.txt"), "new").unwrap();
    fs::write(fx.path("src/sub/a.txt"), "a").unwrap();
    fs::write(fx.path("dst/sub/b.txt"), "b").unwrap();

    merge_path(&fx.path("src"), &fx.path("dst"), &pending()).unwrap();

    assert_eq!(fs::read_to_string(fx.path("dst/new.txt")).unwrap(), "new");
    assert_eq!(fs::read_to_string(fx.path("dst/sub/a.txt")).unwrap(), "a");
    assert_eq!(fs::read_to_string(fx.path("dst/sub/b.txt")).unwrap(), "b");
    assert!(fx.path("src/new.txt").exists(), "merge copies, not moves");
}

#[test]
fn merge_never_replaces_a_colliding_file() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.path("src")).unwrap();
    fs::create_dir_all(fx.path("dst")).unwrap();
    fs::write(fx.path("src/same.txt"), "incoming").unwrap();
    fs::write(fx.path("dst/same.txt"), "original").unwrap();

    let error = merge_path(&fx.path("src"), &fx.path("dst"), &pending()).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(
        fs::read_to_string(fx.path("dst/same.txt")).unwrap(),
        "original"
    );
}

#[test]
fn merge_refuses_a_file_where_a_directory_is_expected() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.path("src")).unwrap();
    fs::write(fx.path("dst"), "not a folder").unwrap();

    let error = merge_path(&fx.path("src"), &fx.path("dst"), &pending()).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(fx.path("dst")).unwrap(), "not a folder");
}

#[test]
fn verified_move_deletes_source_only_after_a_matching_copy() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.path("src/empty")).unwrap();
    fs::write(fx.path("src/file.bin"), vec![7u8; 300_000]).unwrap();

    move_verified(&fx.path("src"), &fx.path("dst"), &pending()).unwrap();

    assert!(!fx.path("src").exists());
    assert_eq!(
        fs::read(fx.path("dst/file.bin")).unwrap(),
        vec![7u8; 300_000]
    );
    assert!(fx.path("dst/empty").is_dir());
}

#[test]
fn verified_move_keeps_source_when_destination_exists() {
    let fx = Fixture::new();
    fs::write(fx.path("a.txt"), "source").unwrap();
    fs::write(fx.path("b.txt"), "other").unwrap();

    let error = move_verified(&fx.path("a.txt"), &fx.path("b.txt"), &pending()).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(fx.path("a.txt")).unwrap(), "source");
    assert_eq!(fs::read_to_string(fx.path("b.txt")).unwrap(), "other");
}

#[test]
fn verification_rejects_a_diverged_copy() {
    let fx = Fixture::new();
    fs::write(fx.path("a.txt"), "abcd").unwrap();
    fs::write(fx.path("b.txt"), "abce").unwrap();
    fs::write(fx.path("c.txt"), "abc").unwrap();

    assert_eq!(
        verify_same(&fx.path("a.txt"), &fx.path("b.txt"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(
        verify_same(&fx.path("a.txt"), &fx.path("c.txt"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn source_deletion_keeps_the_folder_when_a_child_cannot_be_removed() {
    use std::os::windows::fs::OpenOptionsExt;
    let fx = Fixture::new();
    fs::create_dir_all(fx.path("src")).unwrap();
    fs::write(fx.path("src/busy.txt"), "busy").unwrap();
    let busy = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(fx.path("src/busy.txt"))
        .unwrap();

    let result = delete_verified_source(&fx.path("src"));
    drop(busy);

    assert!(result.is_err());
    assert!(fx.path("src/busy.txt").exists());
}

#[test]
fn keep_both_numbers_files_and_directories_without_touching_originals() {
    let fx = Fixture::new();
    fs::write(fx.path("a.txt"), "incoming").unwrap();
    fs::create_dir_all(fx.path("dir")).unwrap();
    fs::write(fx.path("dir/inner.txt"), "inner").unwrap();
    fs::create_dir_all(fx.path("out/dir")).unwrap();
    fs::write(fx.path("out/a.txt"), "first").unwrap();
    fs::write(fx.path("out/a (1).txt"), "second").unwrap();

    let file = copy_path_keep_both(&fx.path("a.txt"), &fx.path("out/a.txt"), &pending()).unwrap();
    let directory = copy_path_keep_both(&fx.path("dir"), &fx.path("out/dir"), &pending()).unwrap();

    assert_eq!(file, fx.path("out/a (2).txt"));
    assert_eq!(directory, fx.path("out/dir (1)"));
    assert_eq!(fs::read_to_string(fx.path("out/a.txt")).unwrap(), "first");
    assert_eq!(
        fs::read_to_string(fx.path("out/a (1).txt")).unwrap(),
        "second"
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), "incoming");
    assert_eq!(
        fs::read_to_string(fx.path("out/dir (1)/inner.txt")).unwrap(),
        "inner"
    );
}

#[test]
fn keep_both_uses_the_requested_name_when_it_is_free() {
    let fx = Fixture::new();
    fs::write(fx.path("a.txt"), "x").unwrap();

    let created = copy_path_keep_both(&fx.path("a.txt"), &fx.path("b.txt"), &pending()).unwrap();

    assert_eq!(created, fx.path("b.txt"));
}
