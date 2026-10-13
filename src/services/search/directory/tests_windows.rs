// SPDX-License-Identifier: MIT

use super::*;

fn indexed_names(root: &std::path::Path, show_hidden: bool) -> Vec<String> {
    let index = SharedIndex::new();
    build_index(
        &index,
        vec![root.to_path_buf()],
        show_hidden,
        usize::MAX,
        Duration::from_secs(10),
        &SearchExclusions::default(),
    );
    let mut names: Vec<_> = index
        .state
        .read()
        .expect("index results")
        .items
        .iter()
        .map(|item| item.name.clone())
        .collect();
    names.sort();
    names
}

#[test]
fn windows_directory_index_obeys_hidden_attribute_not_dot_prefix() {
    let fixture = tempfile::tempdir().expect("scratch directory");
    let hidden = fixture.path().join("hidden.txt");
    std::fs::write(&hidden, b"hidden").expect("write hidden fixture");
    std::fs::write(fixture.path().join(".dot-visible"), b"dot").expect("write dot fixture");
    std::fs::write(fixture.path().join("ordinary.txt"), b"ordinary")
        .expect("write ordinary fixture");
    // Test-only: set actual FILE_ATTRIBUTE_HIDDEN without any UAC.
    let status = std::process::Command::new("attrib.exe")
        .arg("+H")
        .arg(&hidden)
        .status()
        .expect("run native Windows attrib for the fixture");
    assert!(status.success());

    let shown = indexed_names(fixture.path(), false);
    assert_eq!(shown, [".dot-visible", "ordinary.txt"]);
    let all = indexed_names(fixture.path(), true);
    assert_eq!(all, [".dot-visible", "hidden.txt", "ordinary.txt"]);
}
