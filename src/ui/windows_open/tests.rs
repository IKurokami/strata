// SPDX-License-Identifier: MIT

use super::*;
use std::{os::windows::process::ExitStatusExt, process::ExitStatus};

#[test]
fn helper_failure_and_native_crash_are_reported_without_panicking() {
    for (status, stderr, expected) in [
        (
            1,
            "Windows denied this operation",
            "Windows denied this operation",
        ),
        (0xc0000005, "", "Unable to open file"),
    ] {
        let result = helper_result(
            Output {
                status: ExitStatus::from_raw(status),
                stdout: Vec::new(),
                stderr: stderr.as_bytes().to_vec(),
            },
            "Unable to open file",
        );
        assert_eq!(result.unwrap_err().to_string(), expected);
    }
}

#[test]
fn successful_activation_does_not_report_shell_diagnostics_as_failure() {
    assert!(
        helper_result(
            Output {
                status: ExitStatus::from_raw(0),
                stdout: Vec::new(),
                stderr: b"handler diagnostics".to_vec(),
            },
            "Unable to open file",
        )
        .is_ok()
    );
}
