// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn clearing_owned_thumbnails_preserves_unrelated_files_and_never_descends() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    std::fs::create_dir(&cache).unwrap();
    let name = format!("{}.thumb", "a".repeat(64));
    std::fs::write(cache.join(&name), b"cached").unwrap();
    std::fs::write(cache.join("preferences.json"), b"keep").unwrap();
    std::fs::create_dir(cache.join("nested")).unwrap();
    std::fs::write(cache.join("nested").join(&name), b"keep nested").unwrap();
    #[cfg(unix)]
    {
        std::fs::write(root.path().join("original"), b"keep original").unwrap();
        std::os::unix::fs::symlink(
            root.path().join("original"),
            cache.join(format!("{}.thumb", "b".repeat(64))),
        )
        .unwrap();
    }
    clear_thumbnail_files(&cache).unwrap();
    clear_thumbnail_files(&cache).unwrap();
    assert!(!cache.join(&name).exists());
    assert_eq!(
        std::fs::read(cache.join("preferences.json")).unwrap(),
        b"keep"
    );
    assert_eq!(
        std::fs::read(cache.join("nested").join(&name)).unwrap(),
        b"keep nested"
    );
    #[cfg(unix)]
    {
        assert_eq!(
            std::fs::read(root.path().join("original")).unwrap(),
            b"keep original"
        );
        let linked = root.path().join("linked");
        std::os::unix::fs::symlink(&cache, &linked).unwrap();
        assert!(clear_thumbnail_files(&linked).is_err());
    }
}
