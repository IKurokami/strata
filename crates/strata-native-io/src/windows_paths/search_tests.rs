// SPDX-License-Identifier: MIT

use super::*;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "strata-hidden-entry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create fixture");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove fixture");
    }
}

#[test]
fn enumerates_real_windows_hidden_files_and_dotfiles_without_extra_probes() {
    let scratch = Scratch::new();
    let hidden = scratch.0.join("windows-hidden.txt");
    fs::write(&hidden, b"secret").expect("hidden fixture");
    fs::write(scratch.0.join(".visible-dotfile"), b"dot").expect("dotfile fixture");
    fs::write(scratch.0.join("ordinary.txt"), b"visible").expect("ordinary fixture");

    // Test-only invocation: set a genuine Win32 hidden attribute. The
    // production index never launches attrib, shell extensions or processes.
    let status = Command::new("attrib.exe")
        .arg("+H")
        .arg(&hidden)
        .status()
        .expect("Windows attrib.exe available");
    assert!(status.success(), "set the Windows hidden attribute");

    let mut shown = Vec::new();
    let mut hidden_results = Vec::new();
    for entry in fs::read_dir(&scratch.0)
        .expect("enumerate")
        .map(Result::unwrap)
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if should_skip_hidden_entry(&entry, false) {
            hidden_results.push(name.clone());
        }
        if !should_skip_hidden_entry(&entry, false) {
            shown.push(name);
        }
        assert!(!should_skip_hidden_entry(&entry, true));
    }
    shown.sort();
    hidden_results.sort();
    assert_eq!(shown, [".visible-dotfile", "ordinary.txt"]);
    assert_eq!(hidden_results, ["windows-hidden.txt"]);
}
