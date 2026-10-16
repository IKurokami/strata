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
    ffi::{OsStr, c_void},
    io,
    os::windows::ffi::OsStrExt,
    path::Path,
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_BROKEN_PIPE, ERROR_NO_DATA, HANDLE, HANDLE_FLAG_INHERIT,
            HANDLE_FLAGS, SetHandleInformation, TRUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
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
    core::{HRESULT, PCWSTR},
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

// SAFETY: Win32 HANDLE values are process-wide kernel references. Moving the
// owning wrapper between Rust threads does not transfer or duplicate the
// handle, and all access is mediated by the Win32 APIs above.
unsafe impl Send for SafeHandle {}
// SAFETY: The wrapper only closes its handle during Drop; concurrent API calls
// on a live process/job/pipe handle are supported by Win32.
unsafe impl Sync for SafeHandle {}

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

    /// Check if the underlying Job Object handle is valid.
    pub fn is_valid(&self) -> bool {
        self.handle.is_valid()
    }
}

/// Output captured from a sandboxed process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxedOutput {
    pub exit_code: u32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
struct PipeCapture {
    bytes: Vec<u8>,
    truncated: bool,
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

fn make_non_inheritable(handle: HANDLE) -> io::Result<()> {
    // SAFETY: `handle` is an open pipe handle owned by this function's caller.
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }
        .map_err(|error| io::Error::other(format!("SetHandleInformation failed: {error}")))
}

fn handle_from_thread_token(token: usize) -> HANDLE {
    HANDLE(token as *mut c_void)
}

fn read_pipe(handle: HANDLE, truncated_signal: Option<Arc<AtomicBool>>) -> PipeCapture {
    let _handle = SafeHandle::new(handle);
    let mut capture = PipeCapture {
        bytes: Vec::new(),
        truncated: false,
    };
    let mut buffer = [0u8; 8192];

    loop {
        let mut read_bytes = 0u32;
        // SAFETY: The reader thread owns no other handle and the pipe handle remains
        // open until the thread has joined.
        let result = unsafe { ReadFile(handle, Some(&mut buffer), Some(&mut read_bytes), None) };
        if result.is_err() || read_bytes == 0 {
            break;
        }

        let available = MAX_CAPTURE_BYTES.saturating_sub(capture.bytes.len());
        let retained = (read_bytes as usize).min(available);
        capture.bytes.extend_from_slice(&buffer[..retained]);
        capture.truncated |= retained < read_bytes as usize;
        if capture.truncated {
            if let Some(signal) = &truncated_signal {
                signal.store(true, Ordering::Release);
            }
            break;
        }
    }

    capture
}

fn write_pipe(handle: HANDLE, data: &[u8]) {
    let mut offset = 0;
    while offset < data.len() {
        let remaining = &data[offset..];
        let chunk_len = remaining.len().min(u32::MAX as usize);
        let mut written = 0u32;
        // SAFETY: The writer thread owns the pipe handle and `remaining` is valid.
        let result = unsafe {
            WriteFile(
                handle,
                Some(&remaining[..chunk_len]),
                Some(&mut written),
                None,
            )
        };
        if result.is_err() || written == 0 {
            break;
        }
        offset += written as usize;
    }
}

fn terminate_and_wait(job: &JobObject, process: HANDLE) {
    let _ = job.terminate(1);
    for _ in 0..40 {
        // SAFETY: `process` is a valid process handle owned by the caller.
        if unsafe { WaitForSingleObject(process, 25) } == WAIT_OBJECT_0 {
            break;
        }
    }
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
    make_non_inheritable(stdout_read.handle())?;

    let mut stderr_read = HANDLE::default();
    let mut stderr_write = HANDLE::default();
    // SAFETY: Output pointers are valid and sa specifies inheritance.
    unsafe { CreatePipe(&mut stderr_read, &mut stderr_write, Some(&mut sa), 0) }
        .map_err(|e| io::Error::other(format!("CreatePipe stderr failed: {e}")))?;
    let stderr_read = SafeHandle::new(stderr_read);
    let stderr_write = SafeHandle::new(stderr_write);
    make_non_inheritable(stderr_read.handle())?;

    let mut stdin_read = HANDLE::default();
    let mut stdin_write = HANDLE::default();
    if stdin_data.is_some() {
        // SAFETY: Output pointers are valid and sa specifies inheritance.
        unsafe { CreatePipe(&mut stdin_read, &mut stdin_write, Some(&mut sa), 0) }
            .map_err(|e| io::Error::other(format!("CreatePipe stdin failed: {e}")))?;
    }
    let stdin_read = SafeHandle::new(stdin_read);
    let stdin_write = SafeHandle::new(stdin_write);
    if stdin_write.is_valid() {
        make_non_inheritable(stdin_write.handle())?;
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

    let stdout_token = stdout_read.handle().0 as usize;
    let stderr_token = stderr_read.handle().0 as usize;
    std::mem::forget(stdout_read);
    std::mem::forget(stderr_read);
    let stdout_truncated = Arc::new(AtomicBool::new(false));
    let stderr_truncated = Arc::new(AtomicBool::new(false));
    let stdout_reader = {
        let signal = Arc::clone(&stdout_truncated);
        thread::spawn(move || read_pipe(handle_from_thread_token(stdout_token), Some(signal)))
    };
    let stderr_reader = {
        let signal = Arc::clone(&stderr_truncated);
        thread::spawn(move || read_pipe(handle_from_thread_token(stderr_token), Some(signal)))
    };
    let stdin_writer = if let Some(data) = stdin_data {
        let data = data.to_vec();
        let stdin_token = stdin_write.handle().0 as usize;
        std::mem::forget(stdin_write);
        Some(thread::spawn(move || {
            let handle = handle_from_thread_token(stdin_token);
            write_pipe(handle, &data);
            // SAFETY: Ownership of the writer handle was transferred to this thread.
            unsafe {
                let _ = CloseHandle(handle);
            }
        }))
    } else {
        drop(stdin_write);
        None
    };

    // Supervise process with timeout and cancellation checks.
    let start_time = Instant::now();
    let mut failure = None;

    loop {
        if stdout_truncated.load(Ordering::Acquire) || stderr_truncated.load(Ordering::Acquire) {
            failure = Some(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "Sandboxed process output exceeded the capture limit",
            ));
            terminate_and_wait(&job, process_handle.handle());
            break;
        }
        if cancellation.is_some_and(|c| c.load(Ordering::Acquire)) {
            failure = Some(io::Error::new(
                io::ErrorKind::Interrupted,
                "Preview cancelled",
            ));
            terminate_and_wait(&job, process_handle.handle());
            break;
        }

        if start_time.elapsed() >= timeout {
            failure = Some(io::Error::new(
                io::ErrorKind::TimedOut,
                "Sandboxed process exceeded timeout",
            ));
            terminate_and_wait(&job, process_handle.handle());
            break;
        }

        // SAFETY: process_handle is valid.
        let wait_result = unsafe { WaitForSingleObject(process_handle.handle(), 25) };
        if wait_result == WAIT_OBJECT_0 {
            break;
        } else if wait_result != WAIT_TIMEOUT {
            failure = Some(io::Error::other("WaitForSingleObject error"));
            terminate_and_wait(&job, process_handle.handle());
            break;
        }
    }

    let mut exit_code = 0u32;
    // SAFETY: process_handle is valid and terminated.
    unsafe { GetExitCodeProcess(process_handle.handle(), &mut exit_code) }
        .map_err(|e| io::Error::other(format!("GetExitCodeProcess failed: {e}")))?;

    let stdout_capture = stdout_reader
        .join()
        .map_err(|_| io::Error::other("stdout reader thread panicked"))?;
    let stderr_capture = stderr_reader
        .join()
        .map_err(|_| io::Error::other("stderr reader thread panicked"))?;
    if let Some(writer) = stdin_writer {
        let _ = writer.join();
    }

    if let Some(error) = failure {
        return Err(error);
    }
    if stdout_capture.truncated || stderr_capture.truncated {
        return Err(io::Error::new(
            io::ErrorKind::OutOfMemory,
            "Sandboxed process output exceeded the capture limit",
        ));
    }

    Ok(SandboxedOutput {
        exit_code,
        stdout: stdout_capture.bytes,
        stderr: stderr_capture.bytes,
    })
}

/// A child process whose lifetime and descendants are owned by a Job Object.
///
/// The streaming variant is used for media decoders: unlike [`run_in_job`], it
/// does not buffer stdout before returning, so a bounded frame reader can apply
/// back-pressure while the process remains contained. The child is terminated
/// when this value is dropped.
pub struct SandboxedChild {
    job: JobObject,
    process: SafeHandle,
    stdout: SafeHandle,
    stderr_reader: std::sync::Mutex<Option<thread::JoinHandle<PipeCapture>>>,
}

impl SandboxedChild {
    /// Read decoded bytes from the child's stdout pipe.
    pub fn read_stdout(&self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut read_bytes = 0u32;
        // SAFETY: The process owns the read end for the lifetime of this value;
        // the caller supplied a valid writable buffer.
        let result = unsafe {
            ReadFile(
                self.stdout.handle(),
                Some(buffer),
                Some(&mut read_bytes),
                None,
            )
        };
        match result {
            Ok(()) => Ok(read_bytes as usize),
            // A decoder that exits closes its pipe. Win32 reports that normal
            // EOF as ERROR_BROKEN_PIPE (or ERROR_NO_DATA for a raced close).
            Err(error)
                if error.code() == HRESULT::from_win32(ERROR_BROKEN_PIPE.0)
                    || error.code() == HRESULT::from_win32(ERROR_NO_DATA.0) =>
            {
                Ok(0)
            }
            Err(error) => Err(io::Error::other(format!(
                "sandboxed stdout read failed: {error}"
            ))),
        }
    }

    /// Terminate the process tree immediately.
    pub fn terminate(&self) {
        let _ = self.job.terminate(1);
    }

    /// Wait for process termination and drain the stderr reader.
    pub fn wait(&self) {
        // SAFETY: process is a valid handle owned by this value.
        while unsafe { WaitForSingleObject(self.process.handle(), 25) } == WAIT_TIMEOUT {}
        if let Some(reader) = self
            .stderr_reader
            .lock()
            .ok()
            .and_then(|mut reader| reader.take())
        {
            let _ = reader.join();
        }
    }
}

impl Drop for SandboxedChild {
    fn drop(&mut self) {
        self.terminate();
        self.wait();
    }
}

/// Spawn a process suspended, attach it to a Job Object, and return a
/// streaming stdout handle. The process cannot execute before containment is
/// installed, and stderr is drained by a bounded helper thread.
pub fn spawn_streaming_in_job(
    executable: &Path,
    args: &[&OsStr],
    limits: &JobLimits,
) -> io::Result<SandboxedChild> {
    let job = JobObject::create(limits)?;
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        bInheritHandle: TRUE,
        lpSecurityDescriptor: std::ptr::null_mut(),
    };

    let mut stdout_read = HANDLE::default();
    let mut stdout_write = HANDLE::default();
    // SAFETY: output pointers and inheritable security attributes are valid.
    unsafe { CreatePipe(&mut stdout_read, &mut stdout_write, Some(&mut sa), 0) }
        .map_err(|error| io::Error::other(format!("CreatePipe stdout failed: {error}")))?;
    let stdout_read = SafeHandle::new(stdout_read);
    let stdout_write = SafeHandle::new(stdout_write);
    make_non_inheritable(stdout_read.handle())?;

    let mut stderr_read = HANDLE::default();
    let mut stderr_write = HANDLE::default();
    // SAFETY: output pointers and inheritable security attributes are valid.
    unsafe { CreatePipe(&mut stderr_read, &mut stderr_write, Some(&mut sa), 0) }
        .map_err(|error| io::Error::other(format!("CreatePipe stderr failed: {error}")))?;
    let stderr_read = SafeHandle::new(stderr_read);
    let stderr_write = SafeHandle::new(stderr_write);
    make_non_inheritable(stderr_read.handle())?;

    let startup_info = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: HANDLE::default(),
        hStdOutput: stdout_write.handle(),
        hStdError: stderr_write.handle(),
        ..Default::default()
    };
    let mut process_info = PROCESS_INFORMATION::default();
    let mut command_line = build_command_line(executable, args);
    // SAFETY: all pointers reference live handles and a mutable NUL-terminated
    // command line for the duration of CreateProcessW.
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
    let process = SafeHandle::new(process_info.hProcess);
    let thread_handle = SafeHandle::new(process_info.hThread);
    job.assign_process(process.handle())?;
    // SAFETY: the primary thread is valid and was created suspended.
    if unsafe { ResumeThread(thread_handle.handle()) } == u32::MAX {
        let _ = job.terminate(1);
        return Err(io::Error::other("ResumeThread failed"));
    }
    drop(stdout_write);
    drop(stderr_write);

    let stderr_token = stderr_read.handle().0 as usize;
    std::mem::forget(stderr_read);
    let stderr_reader =
        thread::spawn(move || read_pipe(handle_from_thread_token(stderr_token), None));
    Ok(SandboxedChild {
        job,
        process,
        stdout: stdout_read,
        stderr_reader: std::sync::Mutex::new(Some(stderr_reader)),
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

    #[test]
    fn run_in_job_drains_large_output_without_deadlocking() {
        let limits = JobLimits::default();
        let cmd = std::path::Path::new("cmd.exe");
        let args = [
            OsStr::new("/c"),
            OsStr::new("for /L %i in (1,1,100000) do @echo 012345678901234567890123456789"),
        ];
        let result = run_in_job(cmd, &args, None, &limits, Duration::from_secs(20), None);

        assert_eq!(
            result.unwrap_err().kind(),
            io::ErrorKind::OutOfMemory,
            "large untrusted output must be bounded without blocking the child"
        );
    }

    #[test]
    fn streaming_job_keeps_decoder_output_contained() {
        let limits = JobLimits::default();
        let cmd = std::path::Path::new("cmd.exe");
        let args = [OsStr::new("/c"), OsStr::new("echo streamed")];
        let child = spawn_streaming_in_job(cmd, &args, &limits).expect("spawn stream");
        let mut output = Vec::new();
        let mut buffer = [0u8; 64];
        loop {
            let count = child.read_stdout(&mut buffer).expect("read stream");
            if count == 0 {
                break;
            }
            output.extend_from_slice(&buffer[..count]);
        }
        child.wait();
        assert!(String::from_utf8_lossy(&output).contains("streamed"));
    }
}
