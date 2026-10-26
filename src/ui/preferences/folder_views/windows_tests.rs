// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn windows_drive_paths_remember_sorting_after_serialization() {
    let mut views = FolderViews::default();
    for path in [r"C:\Users\owner\Documents", r"E:\Photos", r"C:\Ảnh\Kỷ niệm"] {
        let key = FolderKey::local(Path::new(path)).expect("absolute Windows folder key");
        views.update(&key, |view| {
            view.sort = Some((SortKey::Modified, SortDirection::Descending))
        });
        let (loaded, repaired) =
            FolderViews::parse(&views.to_toml().expect("serialize folder views"))
                .expect("parse folder views");
        assert!(!repaired);
        assert_eq!(
            loaded.view(&key).sort,
            Some((SortKey::Modified, SortDirection::Descending))
        );
    }
    assert!(FolderKey::local(Path::new(r"C:relative")).is_none());
    assert!(FolderKey::local(Path::new(r"C:\folder\..\outside")).is_none());
}
