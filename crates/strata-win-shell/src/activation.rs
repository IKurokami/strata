//! Bounded, current-user and logon-session scoped desktop activation transport.

use std::{
    fs::{File, OpenOptions},
    hash::{Hash, Hasher},
    io::{self, Write},
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{HANDLE, HLOCAL, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                GetSecurityInfo, SE_FILE_OBJECT,
            },
            GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
            SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenSessionId, TokenUser,
        },
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, ReadFile},
        System::{
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
                GetNamedPipeServerProcessId, PIPE_NOWAIT, PIPE_READMODE_BYTE,
                PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, SetNamedPipeHandleState,
            },
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
        UI::WindowsAndMessaging::AllowSetForegroundWindow,
    },
    core::{PCWSTR, PWSTR},
};

const MAX_MESSAGE: usize = 60 * 1024;
const POLL: Duration = Duration::from_millis(10);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(3);

fn win_error(error: windows::core::Error) -> io::Error {
    io::Error::other(error.to_string())
}
fn handle(file: &File) -> HANDLE {
    HANDLE(file.as_raw_handle())
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn read_pipe(file: &File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut count = 0;
    // SAFETY: The owned pipe is synchronous/non-blocking and the buffer is live for this call.
    unsafe { ReadFile(handle(file), Some(buffer), Some(&mut count), None) }
        .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xffff))?;
    // File::read normalizes Windows pipe errors to EOF. Keep NO_DATA distinct
    // from a disconnected peer so acknowledgement and reconnect cannot race.
    Ok(count as usize)
}

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: Windows allocated this security descriptor with LocalAlloc.
        unsafe {
            LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut text = PWSTR::null();
    // SAFETY: The caller retains the token/security descriptor backing this SID.
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.map_err(win_error)?;
    // SAFETY: Conversion returns a valid, terminated UTF-16 allocation.
    let result = unsafe { text.to_string() }.map_err(io::Error::other);
    // SAFETY: Conversion transfers ownership of this LocalAlloc allocation.
    unsafe {
        LocalFree(Some(HLOCAL(text.0.cast())));
    }
    result
}

pub(crate) fn identity() -> io::Result<(String, u32)> {
    let mut raw = HANDLE::default();
    // SAFETY: The current-process pseudo-handle is valid and raw is writable.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) }.map_err(win_error)?;
    // SAFETY: A successful OpenProcessToken transfers an owned kernel handle.
    let token = unsafe { OwnedHandle::from_raw_handle(raw.0) };
    let mut size = 0;
    // SAFETY: A null first buffer queries the required allocation size.
    let _ = unsafe { GetTokenInformation(raw, TokenUser, None, 0, &mut size) };
    let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    // SAFETY: The aligned allocation is at least size bytes and remains live below.
    unsafe {
        GetTokenInformation(
            raw,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            size,
            &mut size,
        )
    }
    .map_err(win_error)?;
    // SAFETY: TokenUser writes a TOKEN_USER followed by the SID into the buffer.
    let sid = sid_string(unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() }.User.Sid)?;
    let mut session = 0u32;
    // SAFETY: TokenSessionId writes one DWORD into the supplied buffer.
    unsafe {
        GetTokenInformation(
            raw,
            TokenSessionId,
            Some((&mut session as *mut u32).cast()),
            4,
            &mut size,
        )
    }
    .map_err(win_error)?;
    drop(token);
    Ok((sid, session))
}

fn endpoint(scope: &str, sid: &str, session: u32) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    scope.hash(&mut hash);
    format!(r"\\.\pipe\Strata-{sid}-{session}-{:016x}", hash.finish())
}

fn verify_owner(file: &File, expected: &str) -> io::Result<()> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let mut owner = PSID::default();
    // SAFETY: The pipe handle includes READ_CONTROL; outputs remain live for conversion.
    unsafe {
        GetSecurityInfo(
            handle(file),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    }
    .ok()
    .map_err(win_error)?;
    let _descriptor = Descriptor(descriptor);
    if sid_string(owner)? != expected {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "activation pipe owner mismatch",
        ));
    }
    Ok(())
}

/// Sends one request and waits for acknowledgement that it entered the primary's queue.
/// No GUI or application data is loaded by this transport.
pub fn forward(scope: &str, message: &[u8]) -> io::Result<()> {
    if message.is_empty() || message.len() > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "activation message size",
        ));
    }
    let (sid, session) = identity()?;
    let path = endpoint(scope, &sid, session);
    let deadline = Instant::now() + CLIENT_TIMEOUT;
    let mut file = loop {
        match OpenOptions::new().access_mode(0xc0020000).open(&path) {
            Ok(file) => break file,
            Err(error) if error.raw_os_error() == Some(231) && Instant::now() < deadline => {
                thread::sleep(POLL)
            }
            Err(error) => return Err(error),
        }
    };
    verify_owner(&file, &sid)?;
    // SAFETY: This is an owned, connected byte-mode pipe; mode is a valid input.
    unsafe { SetNamedPipeHandleState(handle(&file), Some(&PIPE_NOWAIT), None, None) }
        .map_err(win_error)?;
    let mut pid = 0;
    // SAFETY: The connected server's PID is written to a live DWORD.
    if unsafe { GetNamedPipeServerProcessId(handle(&file), &mut pid) }.is_ok() {
        // SAFETY: Granting foreground permission changes no process ownership or memory.
        let _ = unsafe { AllowSetForegroundWindow(pid) };
    }
    let mut frame = (message.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(message);
    file.write_all(&frame)?;
    let deadline = Instant::now() + CLIENT_TIMEOUT;
    loop {
        let mut acknowledgement = [0];
        match read_pipe(&file, &mut acknowledgement) {
            Ok(1) if acknowledgement[0] == 1 => return Ok(()),
            Ok(0) if Instant::now() < deadline => thread::sleep(POLL),
            Err(error) if error.raw_os_error() == Some(232) && Instant::now() < deadline => {
                thread::sleep(POLL)
            }
            Ok(_) => return Err(io::Error::other("activation queue refused request")),
            Err(error) => return Err(error),
        }
    }
}

/// Owns the first pipe instance and a bounded incoming queue. Drop stops its worker.
pub struct Server {
    receiver: Receiver<Vec<u8>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    /// Claims a current-user/session endpoint. An existing primary prevents a second claim.
    pub fn bind(scope: &str) -> io::Result<Self> {
        let (sid, session) = identity()?;
        let name = wide(&endpoint(scope, &sid, session));
        let sddl = wide(&format!("O:{sid}D:P(A;;GA;;;{sid})"));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: SDDL is terminated and outputs are writable; revision 1 is Windows SDDL.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                1,
                &mut descriptor,
                None,
            )
        }
        .map_err(win_error)?;
        let descriptor = Descriptor(descriptor);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0.0,
            bInheritHandle: false.into(),
        };
        // SAFETY: All pointers remain valid for the call; Windows copies the descriptor.
        let raw = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                65536,
                0,
                Some(&attributes),
            )
        };
        if raw.is_invalid() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateNamedPipeW returned a new owned file handle.
        let file = unsafe { File::from_raw_handle(raw.0) };
        let (sender, receiver) = mpsc::sync_channel(16);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::Builder::new()
            .name("strata-activation".into())
            .spawn(move || {
                let mut file = file;
                let mut connected = None;
                let mut buffer = Vec::new();
                while !stopping.load(Ordering::Acquire) {
                    if connected.is_none() {
                        // SAFETY: The owned pipe uses non-blocking mode and no OVERLAPPED.
                        match unsafe { ConnectNamedPipe(handle(&file), None) } {
                            Ok(()) => connected = Some(Instant::now()),
                            Err(error) if error.code().0 as u32 == 0x80070217 => {
                                connected = Some(Instant::now())
                            }
                            Err(error) => {
                                if error.code().0 as u32 == 0x800700e8 {
                                    // SAFETY: A client closed before ConnectNamedPipe; reset this instance.
                                    let _ = unsafe { DisconnectNamedPipe(handle(&file)) };
                                }
                                thread::sleep(POLL);
                                continue;
                            }
                        }
                    }
                    let mut bytes = [0; 4096];
                    let mut reset = false;
                    match read_pipe(&file, &mut bytes) {
                        Ok(0) => {}
                        Ok(count) => buffer.extend_from_slice(&bytes[..count]),
                        Err(error) if matches!(error.raw_os_error(), Some(232 | 536)) => {}
                        Err(_) => reset = true,
                    }
                    if buffer.len() >= 4 {
                        let length =
                            u32::from_le_bytes(buffer[..4].try_into().expect("length prefix"))
                                as usize;
                        if length == 0 || length > MAX_MESSAGE || buffer.len() > length + 4 {
                            reset = true;
                        } else if buffer.len() == length + 4 {
                            let accepted = sender.try_send(buffer[4..].to_vec()).is_ok();
                            let _ = file.write_all(&[u8::from(accepted)]);
                            // Let the client read its acknowledgement before disconnecting.
                            // A byte-mode pipe retains unread server data until the client closes.
                            while connected.is_some_and(|start| start.elapsed() < CLIENT_TIMEOUT)
                                && !stopping.load(Ordering::Acquire)
                            {
                                match read_pipe(&file, &mut bytes) {
                                    Ok(0) => thread::sleep(POLL),
                                    Err(error) if error.raw_os_error() == Some(232) => {
                                        thread::sleep(POLL)
                                    }
                                    _ => break,
                                }
                            }
                            reset = true;
                        }
                    }
                    if connected.is_some_and(|start| start.elapsed() >= CLIENT_TIMEOUT) {
                        reset = true;
                    }
                    if reset {
                        // SAFETY: Disconnect resets only this server's owned pipe instance.
                        let _ = unsafe { DisconnectNamedPipe(handle(&file)) };
                        connected = None;
                        buffer.clear();
                    }
                    thread::sleep(POLL);
                }
            })?;
        Ok(Self {
            receiver,
            stop,
            worker: Some(worker),
        })
    }

    /// Takes the next queued message without blocking the GUI thread.
    pub fn receive(&self) -> Option<Vec<u8>> {
        self.receiver.try_recv().ok()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests;
