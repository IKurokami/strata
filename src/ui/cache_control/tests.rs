// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn clearing_owned_thumbnails_preserves_unrelated_files_and_never_descends() {
    let root = tempfile::tempdir().expect("cache fixture operation");
    let cache = root.path().join("cache");
    std::fs::create_dir(&cache).expect("cache fixture operation");
    let name = format!("{}.thumb", "a".repeat(64));
    std::fs::write(cache.join(&name), b"cached").expect("cache fixture operation");
    std::fs::write(cache.join("preferences.json"), b"keep").expect("cache fixture operation");
    std::fs::create_dir(cache.join("nested")).expect("cache fixture operation");
    std::fs::write(cache.join("nested").join(&name), b"keep nested")
        .expect("cache fixture operation");
    #[cfg(unix)]
    {
        std::fs::write(root.path().join("original"), b"keep original")
            .expect("cache fixture operation");
        std::os::unix::fs::symlink(
            root.path().join("original"),
            cache.join(format!("{}.thumb", "b".repeat(64))),
        )
        .expect("cache fixture operation");
    }
    clear_thumbnail_files(&cache).expect("cache fixture operation");
    clear_thumbnail_files(&cache).expect("cache fixture operation");
    assert!(!cache.join(&name).exists());
    assert_eq!(
        std::fs::read(cache.join("preferences.json")).expect("cache fixture operation"),
        b"keep"
    );
    assert_eq!(
        std::fs::read(cache.join("nested").join(&name)).expect("cache fixture operation"),
        b"keep nested"
    );
    #[cfg(unix)]
    {
        assert_eq!(
            std::fs::read(root.path().join("original")).expect("cache fixture operation"),
            b"keep original"
        );
        let linked = root.path().join("linked");
        std::os::unix::fs::symlink(&cache, &linked).expect("cache fixture operation");
        assert!(clear_thumbnail_files(&linked).is_err());
    }
}
