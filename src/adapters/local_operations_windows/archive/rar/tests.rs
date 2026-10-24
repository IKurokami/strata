// SPDX-License-Identifier: MIT

use super::*;
use std::io::Cursor;

#[test]
fn encrypted_rar_content_and_headers_require_the_correct_password() {
    for (fixture, correct) in [
        ("encrypted.rar", "unrar"),
        ("comment-hpw-password.rar", "password"),
    ] {
        let root = tempfile::tempdir().expect("archive extraction fixture");
        let archive = root.path().join("Ảnh 🔐.rar");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/rar")
                .join(fixture),
            &archive,
        )
        .expect("archive extraction fixture");
        let output = root.path().join("out");
        std::fs::create_dir(&output).expect("archive extraction fixture");
        for (password, expected) in [
            (None, Some(PasswordFailure::Required)),
            (Some("wrong"), Some(PasswordFailure::Incorrect)),
            (Some(correct), None),
        ] {
            let mut wire = Vec::new();
            let _ = crate::sandbox_helper::archive_rar::run(&archive, password, &mut wire);
            let result = extract_stream(
                Cursor::new(wire),
                &output,
                &AtomicUsize::new(0),
                &AtomicBool::new(false),
            );
            if let Some(expected) = expected {
                assert_eq!(
                    result
                        .expect_err("archive extraction must fail")
                        .password_failure(),
                    Some(expected),
                    "{fixture}: {password:?}"
                );
                assert!(
                    std::fs::read_dir(&output)
                        .expect("archive extraction fixture")
                        .next()
                        .is_none()
                );
            } else {
                assert!(result.expect("archive extraction fixture").is_some());
                assert!(
                    std::fs::read_dir(&output)
                        .expect("archive extraction fixture")
                        .next()
                        .is_some()
                );
                if fixture == "comment-hpw-password.rar" {
                    assert_eq!(
                        std::fs::read(output.join(".gitignore"))
                            .expect("archive extraction fixture"),
                        b"target\nCargo.lock\n"
                    );
                }
            }
        }
    }
}

#[test]
fn rar_stream_rejects_escaping_paths_and_removes_failed_members() {
    let root = tempfile::tempdir().expect("archive extraction fixture");
    let mut bytes = Vec::new();
    wire::write_magic(&mut bytes).expect("archive extraction fixture");
    wire::write_file_header(&mut bytes, "../outside", 0, Default::default())
        .expect("archive extraction fixture");
    let result = extract_stream(
        Cursor::new(bytes),
        root.path(),
        &AtomicUsize::new(0),
        &AtomicBool::new(false),
    );
    assert!(result.is_err());
    assert!(
        std::fs::read_dir(root.path())
            .expect("archive extraction fixture")
            .next()
            .is_none()
    );

    let mut bytes = Vec::new();
    wire::write_magic(&mut bytes).expect("archive extraction fixture");
    wire::write_file_header(&mut bytes, "partial.txt", 3, Default::default())
        .expect("archive extraction fixture");
    wire::write_chunk(&mut bytes, b"abc").expect("archive extraction fixture");
    wire::write_file_failed(
        &mut bytes,
        &Failure::new(FailureKind::IncorrectPassword, MAYBE_BAD_PASSWORD),
    )
    .expect("archive extraction fixture");
    let result = extract_stream(
        Cursor::new(bytes),
        root.path(),
        &AtomicUsize::new(0),
        &AtomicBool::new(false),
    );
    assert_eq!(
        result
            .expect_err("archive extraction must fail")
            .password_failure(),
        Some(PasswordFailure::Incorrect)
    );
    assert!(!root.path().join("partial.txt").exists());
}
