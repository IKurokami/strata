use super::*;

#[test]
fn invalid_paths_are_rejected_before_shell_activation() {
    for path in ["relative.exe", "C:\\folder\\bad\0.exe", ""] {
        assert_eq!(
            open(Path::new(path), false).unwrap_err().raw_os_error(),
            Some(87)
        );
    }
}

#[test]
fn missing_executable_returns_an_os_error_without_a_dialog() {
    let path = std::env::temp_dir()
        .join(format!("strata-missing-launch-{}", std::process::id()))
        .join("missing.exe");
    assert!(!path.exists());
    let error = open(&path, false).expect_err("missing executable must fail");
    assert!(matches!(error.raw_os_error(), Some(2 | 3)));
}
