// SPDX-License-Identifier: MIT
use super::*;
use std::{
    fs,
    path::PathBuf,
    process::Child,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FIXTURE: &str = "restart::tests::fixture";

struct Fixture {
    directory: PathBuf,
    parent: Child,
    target: PathBuf,
}

fn until(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "restart fixture timed out"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

impl Fixture {
    fn new() -> Self {
        let token = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("strata-restart-{}-{token}", std::process::id()));
        fs::create_dir(&directory).expect("fixture directory");
        let parent_exe = directory.join("parent ảnh ' test.exe");
        let target = directory.join("child ảnh ' test.exe");
        let current = std::env::current_exe().expect("test executable");
        fs::copy(&current, &parent_exe).expect("parent fixture");
        fs::copy(&current, &target).expect("restart fixture");
        let parent = Command::new(parent_exe)
            .args(["--exact", FIXTURE, "--ignored"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .expect("parent process");
        until(|| directory.join("ready").is_file());
        Self {
            directory,
            parent,
            target,
        }
    }

    fn started(&self) -> u64 {
        creation_time(
            &process(self.parent.id())
                .expect("inspect parent")
                .expect("live parent"),
        )
        .expect("creation time")
    }

    fn arguments() -> Vec<OsString> {
        ["--exact", FIXTURE, "--ignored"]
            .map(OsString::from)
            .to_vec()
    }

    fn released(&mut self) {
        fs::write(self.directory.join("release"), b"done").expect("release parent");
        assert!(self.parent.wait().expect("parent exit").success());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.parent.kill();
        let _ = self.parent.wait();
        let parent = std::env::temp_dir().canonicalize().expect("temporary root");
        if let Ok(directory) = self.directory.canonicalize() {
            assert_eq!(directory.parent(), Some(parent.as_path()));
            assert!(
                directory
                    .file_name()
                    .expect("fixture name")
                    .to_string_lossy()
                    .starts_with("strata-restart-")
            );
            for _ in 0..100 {
                if fs::remove_dir_all(&directory).is_ok() {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[test]
#[ignore = "executed only as a controlled restart subprocess"]
fn fixture() {
    let executable = std::env::current_exe().expect("fixture executable");
    let directory = executable.parent().expect("fixture directory");
    if executable
        .file_name()
        .expect("filename")
        .to_string_lossy()
        .starts_with("parent ")
    {
        fs::write(directory.join("ready"), b"ready").expect("parent ready");
        until(|| directory.join("release").is_file());
    } else {
        fs::write(directory.join("launched"), b"restarted").expect("restart marker");
    }
}

#[test]
fn relaunch_waits_for_parent_exit_and_preserves_unicode_paths() {
    let mut fixture = Fixture::new();
    let pid = fixture.parent.id();
    let started = fixture.started();
    let target = fixture.target.clone();
    let worker = thread::spawn(move || after_exit(pid, started, &target, &Fixture::arguments()));
    thread::sleep(Duration::from_millis(100));
    assert!(
        !fixture.directory.join("launched").exists(),
        "old instance must exit before activation"
    );
    fixture.released();
    worker.join().expect("restart thread").expect("restart");
    until(|| fixture.directory.join("launched").is_file());
    assert_eq!(
        fs::read(fixture.directory.join("launched")).expect("marker"),
        b"restarted"
    );
}

#[test]
fn reused_pid_does_not_wait_for_the_unrelated_process() {
    let mut fixture = Fixture::new();
    after_exit(
        fixture.parent.id(),
        fixture.started() ^ 1,
        &fixture.target,
        &Fixture::arguments(),
    )
    .expect("original instance is gone");
    until(|| fixture.directory.join("launched").is_file());
    assert!(fixture.parent.try_wait().expect("parent status").is_none());
    fixture.released();
}

#[test]
fn invalid_restarts_and_missing_helpers_leave_the_parent_running() {
    let mut fixture = Fixture::new();
    for (pid, started, target) in [
        (0, fixture.started(), fixture.target.as_path()),
        (
            std::process::id(),
            fixture.started(),
            fixture.target.as_path(),
        ),
        (fixture.parent.id(), 0, fixture.target.as_path()),
        (
            fixture.parent.id(),
            fixture.started(),
            Path::new("relative.exe"),
        ),
    ] {
        assert!(after_exit(pid, started, target, &[]).is_err());
    }
    assert!(
        waiter(&fixture.target, fixture.parent.id()).is_err(),
        "missing sibling helper is reported before closing"
    );
    assert!(!fixture.directory.join("launched").exists());
    assert!(fixture.parent.try_wait().expect("parent status").is_none());
    fixture.released();
}
