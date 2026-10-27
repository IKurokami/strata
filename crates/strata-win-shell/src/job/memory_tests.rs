// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn interactive_decoder_reports_committed_memory_after_response() {
    let executable =
        std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("Windows root"))
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let arguments = [
        OsStr::new("-NoLogo"),
        OsStr::new("-NoProfile"),
        OsStr::new("-NonInteractive"),
        OsStr::new("-Command"),
        OsStr::new(
            "[Console]::Out.Write('ready'); [Console]::Out.Flush(); [Console]::ReadLine() | Out-Null",
        ),
    ];
    let child = spawn_interactive_in_job(&executable, &arguments, &JobLimits::default())
        .expect("contained command");
    let mut output = [0; 5];
    child
        .read_stdout_exact(&mut output, Instant::now() + Duration::from_secs(20), None)
        .expect("ready response");
    assert_eq!(&output, b"ready");
    assert!(child.peak_process_memory().expect("memory query") > 0);
}
