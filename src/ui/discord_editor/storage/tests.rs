// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn saves_preserve_bom_and_line_endings_and_reject_external_changes() {
    let folder = tempfile::tempdir().expect("fixture");
    for (name, prefix, line) in [
        ("plain", Vec::new(), "\n"),
        ("bom", vec![0xef, 0xbb, 0xbf], "\r\n"),
    ] {
        let path = folder.path().join(name);
        let mut bytes = prefix.clone();
        bytes.extend_from_slice(format!("ảnh{line}cũ").as_bytes());
        std::fs::write(&path, &bytes).expect("source");
        let snapshot = read(&path).expect("bounded source");
        assert_eq!(snapshot.text, "ảnh\ncũ");
        let saved = write(snapshot, "mới\ntext".into()).expect("save");
        let mut expected = prefix;
        expected.extend_from_slice(format!("mới{line}text").as_bytes());
        assert_eq!(std::fs::read(&path).expect("saved bytes"), expected);
        std::fs::write(&path, "external edit").expect("external edit");
        assert!(write(saved, "overwrite".into()).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).expect("retained external edit"),
            "external edit"
        );
    }
}

#[test]
fn oversized_binary_and_nonregular_files_never_enter_the_editor() {
    let folder = tempfile::tempdir().expect("fixture");
    assert!(read(folder.path()).is_err());
    let path = folder.path().join("source");
    for bytes in [
        vec![b'a'; BYTE_LIMIT + 1],
        vec![0xff, 0xfe, 0x00],
        b"binary\0text".to_vec(),
    ] {
        std::fs::write(&path, bytes).expect("input");
        assert!(read(&path).is_err());
    }
    std::fs::write(&path, "safe").expect("source");
    let snapshot = read(&path).expect("snapshot");
    assert!(write(snapshot, "x".repeat(BYTE_LIMIT + 1)).is_err());
    assert_eq!(std::fs::read_to_string(&path).expect("original"), "safe");
}
