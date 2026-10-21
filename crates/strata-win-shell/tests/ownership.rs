// SPDX-License-Identifier: MIT
#![cfg(windows)]

#[test]
fn startup_drive_classification_does_not_require_the_target_to_exist() {
    let executable = std::env::current_exe().expect("test executable");
    let missing = executable.join("missing-startup-directory");
    assert!(!missing.exists());
    assert!(strata_win_shell::is_fixed_local_drive(&missing));
    for path in ["relative.txt", r"\\server\share\startup-directory"] {
        assert!(!strata_win_shell::is_fixed_local_drive(
            std::path::Path::new(path)
        ));
    }
}

#[test]
fn current_user_files_and_directories_are_owned_but_missing_paths_fail_closed() {
    let root = std::env::temp_dir().join(format!("strata-owner-{}", std::process::id()));
    std::fs::create_dir(&root).expect("ownership fixture directory");
    let file = root.join("provider.json");
    std::fs::write(&file, b"{}").expect("ownership fixture file");
    assert!(strata_win_shell::path_owned_by_current_user(&root));
    assert!(strata_win_shell::path_owned_by_current_user(&file));
    std::fs::remove_file(&file).expect("remove fixture file");
    assert!(!strata_win_shell::path_owned_by_current_user(&file));
    std::fs::remove_dir(&root).expect("remove fixture directory");
}
