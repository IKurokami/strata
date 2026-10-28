// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn root_boundaries_exclude_neighbors_and_sensitive_components() {
    assert!(within(
        Path::new("/pictures/holiday/a.jpg"),
        Path::new("/pictures")
    ));
    assert!(!within(
        Path::new("/pictures-backup/a.jpg"),
        Path::new("/pictures")
    ));
    for path in [
        "node_modules/pkg/a.js",
        "target/debug/app",
        ".git/config",
        ".env.local",
        "credentials",
        "tls.key",
    ] {
        assert!(excluded(Path::new(path)), "{path}");
    }
    assert!(sensitive(Path::new("/home/user/.ssh")));
    assert!(!excluded(Path::new("project/src/main.rs")));
    assert!(code(Path::new("DATA.JSON")));
    assert!(!code(Path::new("readme.txt")));
}

#[cfg(windows)]
#[test]
fn windows_scopes_accept_verbatim_paths_and_case_without_prefix_leaks() {
    assert!(within(
        Path::new(r"\\?\C:\Users\Name\Docs\a.txt"),
        Path::new(r"c:\users\name\docs")
    ));
    assert!(!within(
        Path::new(r"C:\Docs-old\a.txt"),
        Path::new(r"C:\Docs")
    ));
}
