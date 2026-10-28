// SPDX-License-Identifier: MIT
#![cfg(windows)]

#[test]
fn directory_identity_survives_rename_and_distinguishes_a_replacement() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-identity-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).expect("identity fixture");
    let original = root.join("thư mục");
    let renamed = root.join("renamed");
    std::fs::create_dir(&original).expect("original directory");
    let identity =
        strata_win_shell::directory_identity(&original).expect("native directory identity");
    std::fs::rename(&original, &renamed).expect("identity inspection leaves no blocking handle");
    assert_eq!(
        strata_win_shell::directory_identity(&renamed).expect("renamed identity"),
        identity
    );
    std::fs::create_dir(&original).expect("replacement directory");
    assert_ne!(
        strata_win_shell::directory_identity(&original).expect("replacement identity"),
        identity
    );
    assert!(strata_win_shell::directory_identity(&root.join("missing")).is_err());
    for path in [&original, &renamed, &root] {
        std::fs::remove_dir(path).expect("remove identity fixture");
    }
}

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
