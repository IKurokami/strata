// SPDX-License-Identifier: MIT
use super::*;

#[test]
fn upgrades_read_saved_intent_and_reject_invalid_or_oversized_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.toml");
    assert_eq!(read(&path), None);
    for (content, expected) in [
        ("replace_file_explorer = false", Some(false)),
        ("replace_file_explorer = true", Some(true)),
        ("replace_file_explorer = 'true'", None),
        ("invalid = [", None),
        ("other = false", None),
    ] {
        std::fs::write(&path, content).unwrap();
        assert_eq!(read(&path), expected);
    }
    std::fs::write(
        &path,
        format!("replace_file_explorer = true\n#{}", "x".repeat(1024 * 1024)),
    )
    .unwrap();
    assert_eq!(read(&path), None);
}
