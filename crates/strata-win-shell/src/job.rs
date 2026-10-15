// SPDX-License-Identifier: MIT
//! Win32 Job Object process boundary for untrusted sandbox decoders.
//!
//! Provides strict process isolation:
//! - Hard process limit (cannot spawn child processes / fork-bomb).
//! - Hard memory limit (cannot exhaust host memory).
//! - Automatic process tree termination on job close or timeout.
//! - UI restrictions (cannot access external window handles, clipboard, or desktop).
#![cfg(windows)]

use std::{
    ffi::OsStr,
    io,
    os::windows::ffi::OsStrExt,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, TRUE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::{ReadFile, WriteFile},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
                JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_UILIMIT, JOB_OBJECT_UILIMIT_DESKTOP,
                JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
                JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES,
                JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
                JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_LIMIT_INFORMATION,
                JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation,
                SetInformationJobObject, TerminateJobObject,
            },
            Pipes::CreatePipe,
            Threading::{
                CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessW, GetExitCodeProcess,
                PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOW,
                WaitForSingleObject,
            },
        },
    },
    core::PCWSTR,
};

/// RAII wrapper for Win32 `HANDLE`.
#[derive(Debug)]
pub struct SafeHandle(HANDLE);

impl SafeHandle {
    pub fn new(handle: HANDLE) -> Self {
        Self(handle)
    }

    pub fn handle(&self) -> HANDLE {
        self.0
    }

    pub fn is_valid(&self) -> bool {
        !self.0.is_invalid() && self.0.0 != std::ptr::null_mut()
    }
}

impl Drop for SafeHandle {
    fn drop(&mut self) {
        if self.is_valid() {
            // SAFETY: Valid open Win32 handle owned exclusively by this wrapper.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Limits applied to the sandboxed Job Object.
#[derive(Clone, Debug)]
pub struct JobLimits {
    /// Maximum committed memory per process in bytes (default: 1 GiB).
    pub memory_limit_bytes: Option<usize>,
    /// Maximum active processes in the job (default: 1, prevents subprocess spawning).
    pub active_process_limit: Option<u32>,
    /// Terminate all processes when the job object handle closes.
    pub kill_on_job_close: bool,
    /// Apply desktop, clipboard, and handle UI restrictions.
    pub restrict_ui: bool,
}

impl Default for JobLimits {
    fn default() -> Self {
        Self {
            memory_limit_bytes: Some(1024 * 1024 * 1024),
            active_process_limit: Some(1),
            kill_on_job_close: true,
            restrict_ui: true,
        }
    }
}

/// A Win32 Job Object with configured sandbox boundaries.
pub struct JobObject {
    handle: SafeHandle,
}

impl JobObject {
    /// Create a new Job Object and apply the requested limits.
    pub fn create(limits: &JobLimits) -> io::Result<Self> {
        // SAFETY: Standard anonymous Job Object creation.
        let job = unsafe { CreateJobObjectW(None, None) }
            .map_err(|error| io::Error::other(format!("CreateJobObjectW failed: {error}")))?;
        let job_handle = SafeHandle::new(job);

        let mut limit_flags = 0u32;
        if limits.kill_on_job_close {
            limit_flags |= JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.0;
        }
        if limits.active_process_limit.is_some() {
            limit_flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS.0;
        }
        if limits.memory_limit_bytes.is_some() {
            limit_flags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY.0;
        }
        limit_flags |= JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION.0;

        let mut extended_limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                LimitFlags: windows::Win32::System::JobObjects::JOB_OBJECT_LIMIT(limit_flags),
                ActiveProcessLimit: limits.active_process_limit.unwrap_or(0),
                ..Default::default()
            },
            ProcessMemoryLimit: limits.memory_limit_bytes.unwrap_or(0),
            ..Default::default()
        };

        // SAFETY: `extended_limits` is valid and its size matches the information class.
        let success = unsafe {
            SetInformationJobObject(
                job_handle.handle(),
                JobObjectExtendedLimitInformation,
                (&raw mut extended_limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if let Err(error) = success {
            return Err(io::Error::other(format!(
                "SetInformationJobObject (extended limits) failed: {error}"
            )));
        }

        if limits.restrict_ui {
            let mut ui_restrictions = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                UIRestrictionsClass: JOB_OBJECT_UILIMIT(
                    JOB_OBJECT_UILIMIT_HANDLES.0
                        | JOB_OBJECT_UILIMIT_READCLIPBOARD.0
                        | JOB_OBJECT_UILIMIT_WRITECLIPBOARD.0
                        | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS.0
                        | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS.0
                        | JOB_OBJECT_UILIMIT_GLOBALATOMS.0
                        | JOB_OBJECT_UILIMIT_DESKTOP.0
                        | JOB_OBJECT_UILIMIT_EXITWINDOWS.0,
                ),
            };

            // SAFETY: `ui_restrictions` is valid and its size matches the information class.
            let success = unsafe {
                SetInformationJobObject(
                    job_handle.handle(),
                    JobObjectBasicUIRestrictions,
                    (&raw mut ui_restrictions).cast(),
                    std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
                )
            };
            if let Err(error) = success {
                return Err(io::Error::other(format!(
                    "SetInformationJobObject (UI restrictions) failed: {error}"
                )));
            }
        }

        Ok(Self { handle: job_handle })
    }

    /// Assign an existing process to this Job Object.
    pub fn assign_process(&self, process: HANDLE) -> io::Result<()> {
        // SAFETY: Both job and process handles are valid.
        unsafe { AssignProcessToJobObject(self.handle.handle(), process) }
            .map_err(|error| io::Error::other(format!("AssignProcessToJobObject failed: {error}")))
    }

    /// Terminate all processes in this Job Object immediately.
    pub fn terminate(&self, exit_code: u32) -> io::Result<()> {
        // SAFETY: Valid job handle.
        unsafe { TerminateJobObject(self.handle.handle(), exit_code) }
            .map_err(|error| io::Error::other(format!("TerminateJobObject failed: {error}")))
    }
}

/// Output captured from a sandboxed process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxedOutput {
    pub exit_code: u32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

fn to_wide_null(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn quote_windows_arg(arg: &OsStr) -> Vec<u16> {
    let s = arg.to_string_lossy();
    let mut quoted = String::new();
    if !s.contains([' ', '\t', '\n', '"']) && !s.is_empty() {
        quoted.push_str(&s);
    } else {
        quoted.push('"');
        let mut backslashes = 0;
        for c in s.chars() {
            if c == '\\' {
                backslashes += 1;
            } else if c == '"' {
                for _ in 0..backslashes * 2 + 1 {
                    quoted.push('\\');
                }
                quoted.push('"');
                backslashes = 0;
            } else {
                for _ in 0..backslashes {
                    quoted.push('\\');
                }
                backslashes = 0;
                quoted.push(c);
            }
        }
        for _ in 0..backslashes * 2 {
            quoted.push('\\');
        }
        quoted.push('"');
    }
    to_wide_null(OsStr::new(&quoted))
}

fn build_command_line(executable: &Path, args: &[&OsStr]) -> Vec<u16> {
    let mut cmd = quote_windows_arg(executable.as_os_str());
    // Remove null terminator before appending next arguments
    cmd.pop();
    for arg in args {
        cmd.push(b' ' as u16);
        let mut quoted = quote_windows_arg(arg);
        quoted.pop();
        cmd.extend(quoted);
    }
    cmd.push(0);
    cmd
}

/// Execute a command in a dedicated, isolated Job Object with timeout and cancellation.
pub fn run_in_job(
    executable: &Path,
    args: &[&OsStr],
    stdin_data: Option<&[u8]>,
    limits: &JobLimits,
    timeout: Duration,
    cancellation: Option<&AtomicBool>,
) -> io::Result<SandboxedOutput> {
    let job = JobObject::create(limits)?;

    // Create pipes for stdout and stderr, and optionally stdin.
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        bInheritHandle: TRUE,
        lpSecurityDescriptor: std::ptr::null_mut(),
    };

    let mut stdout_read = HANDLE::default();
    let mut stdout_write = HANDLE::default();
    // SAFETY: Output pointers are valid and sa specifies inheritance.
    unsafe { CreatePipe(&mut stdout_read, &mut stdout_write, Some(&mut sa), 0) }
        .map_err(|e| io::Error::other(format!("CreatePipe stdout failed: {e}")))?;
    let stdout_read = SafeHandle::new(stdout_read);
    let stdout_write = SafeHandle::new(stdout_write);

    let mut stderr_read = HANDLE::default();
    let mut stderr_write = HANDLE::default();
    // SAFETY: Output pointers are valid and sa specifies inheritance.
    unsafe { CreatePipe(&mut stderr_read, &mut stderr_write, Some(&mut sa), 0) }
        .map_err(|e| io::Error::other(format!("CreatePipe stderr failed: {e}")))?;
    let stderr_read = SafeHandle::new(stderr_read);
    let stderr_write = SafeHandle::new(stderr_write);

    let mut stdin_read = HANDLE::default();
    let mut stdin_write = HANDLE::default();
    if stdin_data.is_some() {
        // SAFETY: Output pointers are valid and sa specifies inheritance.
        unsafe { CreatePipe(&mut stdin_read, &mut stdin_write, Some(&mut sa), 0) }
            .map_err(|e| io::Error::other(format!("CreatePipe stdin failed: {e}")))?;
    }
    let stdin_read = SafeHandle::new(stdin_read);
    let stdin_write = SafeHandle::new(stdin_write);

    // Write stdin data and close write end so child receives EOF
    if let Some(data) = stdin_data {
        if !data.is_empty() {
            let mut written = 0u32;
            // SAFETY: stdin_write is valid, data buffer is valid.
            let _ =
                unsafe { WriteFile(stdin_write.handle(), Some(data), Some(&mut written), None) };
        }
        drop(stdin_write);
    }

    let startup_info = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: if stdin_read.is_valid() {
            stdin_read.handle()
        } else {
            HANDLE::default()
        },
        hStdOutput: stdout_write.handle(),
        hStdError: stderr_write.handle(),
        ..Default::default()
    };

    let mut process_info = PROCESS_INFORMATION::default();
    let mut command_line = build_command_line(executable, args);

    // Spawn suspended so we can assign to Job Object before any instructions execute.
    // SAFETY: Pointers are valid, handles are prepared.
    let created = unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(windows::core::PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_SUSPENDED | CREATE_NO_WINDOW,
            None,
            PCWSTR::null(),
            &startup_info,
            &mut process_info,
        )
    };
    if let Err(error) = created {
        return Err(io::Error::other(format!("CreateProcessW failed: {error}")));
    }

    let process_handle = SafeHandle::new(process_info.hProcess);
    let thread_handle = SafeHandle::new(process_info.hThread);

    // Assign to Job Object while still suspended.
    if let Err(error) = job.assign_process(process_handle.handle()) {
        let _ = job.terminate(1);
        return Err(error);
    }

    // Resume execution inside the Job Object.
    // SAFETY: Thread handle is valid and suspended.
    let resumed = unsafe { ResumeThread(thread_handle.handle()) };
    if resumed == u32::MAX {
        let _ = job.terminate(1);
        return Err(io::Error::other("ResumeThread failed"));
    }

    // Close our copy of the write pipe ends so read pipes will see EOF on process exit.
    drop(stdout_write);
    drop(stderr_write);
    drop(stdin_read);

    // Supervise process with timeout and cancellation checks.
    let start_time = Instant::now();

    loop {
        if cancellation.is_some_and(|c| c.load(Ordering::Acquire)) {
            let _ = job.terminate(1);
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Preview cancelled",
            ));
        }

        if start_time.elapsed() >= timeout {
            let _ = job.terminate(1);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Sandboxed process exceeded timeout",
            ));
        }

        // SAFETY: process_handle is valid.
        let wait_result = unsafe { WaitForSingleObject(process_handle.handle(), 25) };
        if wait_result == WAIT_OBJECT_0 {
            break;
        } else if wait_result != WAIT_TIMEOUT {
            let _ = job.terminate(1);
            return Err(io::Error::other("WaitForSingleObject error"));
        }
    }

    let mut exit_code = 0u32;
    // SAFETY: process_handle is valid and terminated.
    unsafe { GetExitCodeProcess(process_handle.handle(), &mut exit_code) }
        .map_err(|e| io::Error::other(format!("GetExitCodeProcess failed: {e}")))?;

    // Read remaining stdout and stderr.
    let mut stdout = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let mut read_bytes = 0u32;
        // SAFETY: ReadFile on valid pipe handle.
        let ok = unsafe {
            ReadFile(
                stdout_read.handle(),
                Some(&mut buffer),
                Some(&mut read_bytes),
                None,
            )
        };
        if ok.is_err() || read_bytes == 0 {
            break;
        }
        stdout.extend_from_slice(&buffer[..read_bytes as usize]);
    }

    let mut stderr = Vec::new();
    loop {
        let mut read_bytes = 0u32;
        // SAFETY: ReadFile on valid pipe handle.
        let ok = unsafe {
            ReadFile(
                stderr_read.handle(),
                Some(&mut buffer),
                Some(&mut read_bytes),
                None,
            )
        };
        if ok.is_err() || read_bytes == 0 {
            break;
        }
        stderr.extend_from_slice(&buffer[..read_bytes as usize]);
    }

    Ok(SandboxedOutput {
        exit_code,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_object_creation_and_limits() {
        let limits = JobLimits::default();
        let job = JobObject::create(&limits).expect("create job object");
        assert!(job.handle.is_valid());
    }

    #[test]
    fn run_in_job_executes_and_terminates() {
        let limits = JobLimits::default();
        let cmd = std::path::Path::new("cmd.exe");
        let args = [OsStr::new("/c"), OsStr::new("echo hello sandbox")];
        let output = run_in_job(cmd, &args, None, &limits, Duration::from_secs(5), None)
            .expect("run cmd in job");

        assert_eq!(output.exit_code, 0);
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("hello sandbox"), "got output: {text}");
    }

    #[test]
    fn run_in_job_respects_cancellation() {
        let limits = JobLimits::default();
        let cmd = std::path::Path::new("cmd.exe");
        let args = [OsStr::new("/c"), OsStr::new("timeout 10")];
        let cancelled = AtomicBool::new(true);
        let result = run_in_job(
            cmd,
            &args,
            None,
            &limits,
            Duration::from_secs(5),
            Some(&cancelled),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }
}
