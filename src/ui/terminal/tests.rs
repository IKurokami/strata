// SPDX-License-Identifier: MIT

use super::*;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
#[test]
fn reap_in_background_collects_an_exited_child() -> std::io::Result<()> {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "exit 0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = command.spawn()?;
    let pid = child.id();

    reap_in_background(child, command.get_program().to_owned());

    let parent = format!("PPid:\t{}\n", std::process::id());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
            return Ok(());
        };
        if !status.contains(&parent) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let state = status
                .lines()
                .find(|line| line.starts_with("State:"))
                .unwrap_or("State: unknown");
            panic!("child {pid} was not reaped; last {state}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(windows)]
#[test]
fn windows_terminal_receives_directory_as_an_argument_not_shell_code() {
    let terminal = Terminal::known(OsString::from("wt.exe"), &KNOWN_TERMINALS[0]);
    let path = Path::new(r"C:\folder with spaces & symbols");
    let command = terminal.directory_command(path);
    assert_eq!(command.get_current_dir(), Some(path));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![OsStr::new("-d"), path.as_os_str()]
    );
}

#[cfg(windows)]
#[test]
fn powershell_and_cmd_inherit_working_directory_without_shell_expansion() {
    let path = Path::new(r"C:\work & personal");
    for known in &KNOWN_TERMINALS[1..] {
        let terminal = Terminal::known(OsString::from(known.program), known);
        let command = terminal.directory_command(path);
        assert_eq!(command.get_current_dir(), Some(path));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            known.leading
                .iter()
                .map(OsStr::new)
                .collect::<Vec<_>>()
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_known_terminals_use_only_native_executables() {
    assert_eq!(PREFERRED_LAUNCHER, "wt.exe");
    assert_eq!(
        KNOWN_TERMINALS.iter().map(|item| item.program).collect::<Vec<_>>(),
        vec!["wt.exe", "pwsh.exe", "powershell.exe", "cmd.exe"]
    );
}
