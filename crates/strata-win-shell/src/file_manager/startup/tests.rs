// SPDX-License-Identifier: MIT

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture {
    key: String,
    directory: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let token = format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let directory = std::env::temp_dir().join(format!("strata-startup-{token}"));
        std::fs::create_dir_all(&directory).expect("private directory");
        Self {
            key: format!(r"Software\Strata\Tests\Startup\{token}"),
            directory,
        }
    }
    fn executable(&self, folder: &str) -> PathBuf {
        let folder = self.directory.join(folder);
        std::fs::create_dir_all(&folder).expect("fixture folder");
        let executable = folder.join("strata.exe");
        std::fs::write(&executable, b"test fixture").expect("fixture exe");
        executable
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = registry::remove_tree(&self.key);
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn startup_enable_disable_is_owned_idempotent_and_preserves_other_entries() {
    let fixture = Fixture::new();
    let executable = fixture.executable("Bản thử 中文");
    let other = Value::text(std::ffi::OsStr::new("foreign command")).expect("foreign value");
    registry::set(&fixture.key, "OtherApplication", Some(&other)).expect("foreign entry");
    apply(&fixture.key, &executable, true).expect("enable startup");
    apply(&fixture.key, &executable, true).expect("repeat enable");
    assert_eq!(
        registered_executable(
            &registry::get(&fixture.key, NAME)
                .expect("query")
                .expect("entry")
        ),
        Some(executable.clone())
    );
    std::fs::remove_file(&executable).expect("simulate removed executable");
    apply(&fixture.key, &executable, false).expect("cleanup removed installation");
    apply(&fixture.key, &executable, false).expect("repeat disable");
    assert!(registry::get(&fixture.key, NAME).expect("query").is_none());
    assert_eq!(
        registry::get(&fixture.key, "OtherApplication").expect("query"),
        Some(other)
    );
}

#[test]
fn startup_updates_strata_paths_but_preserves_foreign_commands_and_newer_owners() {
    let fixture = Fixture::new();
    let old = fixture.executable("old");
    let current = fixture.executable("new");
    apply(&fixture.key, &old, true).expect("old install");
    apply(&fixture.key, &current, true).expect("moved install");
    apply(&fixture.key, &old, false).expect("uninstall old");
    assert_eq!(
        registered_executable(
            &registry::get(&fixture.key, NAME)
                .expect("query")
                .expect("entry")
        ),
        Some(current.clone())
    );
    let foreign = Value::text(std::ffi::OsStr::new("\"C:\\Other\\app.exe\" --custom"))
        .expect("foreign command");
    registry::set(&fixture.key, NAME, Some(&foreign)).expect("external change");
    assert!(apply(&fixture.key, &current, true).is_err());
    apply(&fixture.key, &current, false).expect("preserve foreign registration");
    assert_eq!(
        registry::get(&fixture.key, NAME).expect("query"),
        Some(foreign)
    );
}
