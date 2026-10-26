use super::*;
use crate::tests::Fixture;

#[test]
fn parallel_subtrees_are_removed_without_touching_siblings() {
    let fixture = Fixture::new();
    let target = fixture.0.join("target");
    let sibling = fixture.0.join("keep.txt");
    fs::write(&sibling, b"keep").expect("sibling");
    for index in 0..64 {
        let directory = target.join(format!("package-{index}/nested"));
        fs::create_dir_all(&directory).expect("directory");
        fs::write(directory.join("file.txt"), b"content").expect("file");
    }
    remove(&target, &AtomicBool::new(false)).expect("parallel deletion");
    assert!(!target.exists());
    assert_eq!(fs::read(sibling).expect("sibling preserved"), b"keep");
}

#[test]
fn native_filenames_are_not_converted_to_utf8() {
    let fixture = Fixture::new();
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![b'f', 0xff])
    };
    #[cfg(windows)]
    let name = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[0x66, 0xd800])
    };
    let target = fixture.0.join(name);
    fs::write(&target, b"content").expect("native filename");
    remove(&target, &AtomicBool::new(false)).expect("native file removal");
    assert!(!target.exists());
}

#[test]
fn cancellation_before_dispatch_preserves_the_tree() {
    let fixture = Fixture::new();
    let target = fixture.0.join("target");
    fs::create_dir(&target).expect("directory");
    let file = target.join("keep.txt");
    fs::write(&file, b"keep").expect("file");
    assert_eq!(
        remove(&target, &AtomicBool::new(true))
            .expect_err("cancelled")
            .kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(fs::read(file).expect("untouched file"), b"keep");
}

#[test]
fn directory_links_are_unlinked_without_traversing_the_target() {
    for nested in [false, true] {
        let fixture = Fixture::new();
        let outside = fixture.0.join("outside");
        fs::create_dir(&outside).expect("outside directory");
        fs::write(outside.join("keep.txt"), b"keep").expect("outside file");
        let target = fixture.0.join("target");
        let link = if nested {
            fs::create_dir(&target).expect("target directory");
            target.join("link")
        } else {
            target.clone()
        };
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).expect("directory symlink");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .creation_flags(0x0800_0000)
                .output()
                .expect("junction creation");
            assert!(
                output.status.success(),
                "junction creation failed: {:?}",
                output
            );
        }
        remove(&target, &AtomicBool::new(false)).expect("unlink directory link");
        assert!(!target.exists());
        assert_eq!(
            fs::read(outside.join("keep.txt")).expect("outside preserved"),
            b"keep"
        );
    }
}

#[cfg(windows)]
#[test]
fn sharing_failures_are_reported_and_leave_the_locked_file_intact() {
    use std::os::windows::fs::OpenOptionsExt;
    let fixture = Fixture::new();
    let target = fixture.0.join("target");
    fs::create_dir(&target).expect("target directory");
    let file = target.join("locked.txt");
    fs::write(&file, b"keep").expect("file");
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&file)
        .expect("deny sharing");
    assert!(remove(&target, &AtomicBool::new(false)).is_err());
    drop(held);
    assert_eq!(fs::read(file).expect("locked file preserved"), b"keep");
}
