// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn missing_optional_attributes_use_safe_defaults() {
    crate::test_support::gtk_test_with_env(
        "adapters::local_files::metadata_tests::missing_optional_attributes_use_safe_defaults",
        [("G_DEBUG", "fatal-criticals")],
        || {
            let info = gio::FileInfo::new();
            assert!(!info_is_hidden(&info));
            assert!(!info_is_symlink(&info));
            assert_eq!(info_mode(&info), MetadataValue::Unavailable);
            info.set_is_hidden(true);
            info.set_is_symlink(true);
            assert!(info_is_hidden(&info));
            assert!(info_is_symlink(&info));
            info.set_attribute_uint32(gio::FILE_ATTRIBUTE_UNIX_MODE, 0);
            assert_eq!(info_mode(&info), MetadataValue::Known(0));

            let location = Location::local(std::env::temp_dir().join("strata-size-fixture"));
            for (kind, size, expected) in [
                (gio::FileType::Regular, None, MetadataValue::Unavailable),
                (gio::FileType::Regular, Some(-1), MetadataValue::Unavailable),
                (gio::FileType::Regular, Some(0), MetadataValue::Known(0)),
                (gio::FileType::Regular, Some(7), MetadataValue::Known(7)),
                (gio::FileType::Directory, None, MetadataValue::Unknown),
            ] {
                let info = gio::FileInfo::new();
                info.set_file_type(kind);
                if let Some(size) = size {
                    info.set_size(size);
                }
                let (update, _) = update_from_info(&info, &location);
                assert_eq!(update.size, expected, "{kind:?}: {size:?}");
            }
        },
    );
}
