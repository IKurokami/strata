// SPDX-License-Identifier: MIT
//! Wait for a specific application instance to exit before relaunching it.

use std::{
    ffi::OsString,
    io,
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::Path,
    process::{Command, Stdio},
};
use windows::{
    Win32::{
        Foundation::{ERROR_INVALID_PARAMETER, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            CREATE_NO_WINDOW, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_SYNCHRONIZE, WaitForSingleObject,
        },
    },
    core::HRESULT,
};

fn process(pid: u32) -> io::Result<Option<OwnedHandle>> {
    // SAFETY: OpenProcess returns an independently owned handle, never a pseudo handle.
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    };
    match handle {
        Ok(handle) => {
            // SAFETY: Ownership transfers exactly once and OwnedHandle closes it.
            Ok(Some(unsafe { OwnedHandle::from_raw_handle(handle.0) }))
        }
        Err(error) if error.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => Ok(None),
        Err(error) => Err(io::Error::other(error)),
    }
}

fn creation_time(process: &OwnedHandle) -> io::Result<u64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: The handle is live and every output points to writable FILETIME storage.
    unsafe {
        GetProcessTimes(
            HANDLE(process.as_raw_handle()),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    }
    .map_err(io::Error::other)?;
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

/// Construct the installed sibling helper without searching PATH or using a shell.
pub fn waiter(executable: &Path, parent_pid: u32) -> io::Result<Command> {
    let parent = process(parent_pid)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Restart parent has exited"))?;
    let directory = executable
        .parent()
        .filter(|_| executable.is_absolute() && executable.is_file())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid restart executable"))?;
    let helper = directory.join("strata-shell.exe");
    if !helper.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Strata restart helper is missing",
        ));
    }
    let mut command = Command::new(helper);
    command
        .arg("--restart")
        .arg(parent_pid.to_string())
        .arg(creation_time(&parent)?.to_string())
        .arg(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0);
    Ok(command)
}

/// Relaunch only after the identified parent exits. A reused PID is not that parent.
pub fn after_exit(
    parent_pid: u32,
    started: u64,
    executable: &Path,
    arguments: &[OsString],
) -> io::Result<()> {
    if parent_pid == 0
        || parent_pid == std::process::id()
        || started == 0
        || !executable.is_absolute()
        || !executable.is_file()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid restart request",
        ));
    }
    if let Some(parent) = process(parent_pid)?
        && creation_time(&parent)? == started
    {
        // SAFETY: The owned process handle stays live for the bounded wait.
        match unsafe { WaitForSingleObject(HANDLE(parent.as_raw_handle()), 120_000) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Restart parent is still running",
                ));
            }
            _ => return Err(io::Error::last_os_error()),
        }
    }
    Command::new(executable)
        .args(arguments)
        .current_dir(executable.parent().expect("absolute file parent"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests;
