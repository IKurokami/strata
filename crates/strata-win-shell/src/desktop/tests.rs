// SPDX-License-Identifier: MIT

use super::*;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use windows::Win32::Foundation::HANDLE;

#[test]
fn allocated_console_replays_startup_logs_and_preserves_redirected_output() {
    const CHILD: &str = "STRATA_CONSOLE_REGRESSION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "desktop::tests::allocated_console_replays_startup_logs_and_preserves_redirected_output", "--nocapture"])
            .env(CHILD, "1")
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("redirected stream remains usable")
        );
        return;
    }
    // SAFETY: Only this isolated child detaches from any inherited console.
    unsafe {
        let _ = FreeConsole();
    }
    assert!(!write_diagnostics(b"startup history marker\r\n"));
    let mut console = Console::default();
    // SAFETY: Query the child's borrowed redirected output handle.
    let redirected = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }.unwrap();
    console.set_visible(true).unwrap();
    // SAFETY: This only queries the child's borrowed standard output handle.
    let after = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }.unwrap();
    assert_eq!(after, redirected);
    assert!(!write_diagnostics(b"live diagnostic marker\r\n"));
    console.set_visible(false).unwrap();
    assert!(!write_diagnostics(b"hidden diagnostic marker\r\n"));
    console.set_visible(true).unwrap();
    let screen = OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$")
        .unwrap();
    let mut text = vec![0u16; 4096];
    let mut read = 0;
    // SAFETY: The owned console handle and writable UTF-16 buffer remain live.
    unsafe {
        ReadConsoleOutputCharacterW(
            HANDLE(screen.as_raw_handle()),
            &mut text,
            COORD { X: 0, Y: 0 },
            &mut read,
        )
    }
    .unwrap();
    let text = String::from_utf16_lossy(&text[..read as usize]);
    for marker in [
        "startup history marker",
        "live diagnostic marker",
        "hidden diagnostic marker",
    ] {
        assert_eq!(text.matches(marker).count(), 1, "{text}");
    }
    let mut duplicate = HANDLE::default();
    // SAFETY: Duplicate the output handle so the test does not close the borrowed original.
    unsafe {
        windows::Win32::Foundation::DuplicateHandle(
            windows::Win32::System::Threading::GetCurrentProcess(),
            redirected,
            windows::Win32::System::Threading::GetCurrentProcess(),
            &mut duplicate,
            0,
            false,
            windows::Win32::Foundation::DUPLICATE_SAME_ACCESS,
        )
    }
    .unwrap();
    // SAFETY: DuplicateHandle transferred this independent handle to the child.
    let mut output = unsafe { File::from_raw_handle(duplicate.0) };
    output
        .write_all(b"redirected stream remains usable\n")
        .unwrap();
}
