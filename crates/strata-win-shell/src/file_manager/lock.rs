// SPDX-License-Identifier: MIT

use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use windows::{
    Win32::{
        Foundation::{HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0},
        System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    },
    core::PCWSTR,
};

pub(super) struct Guard(OwnedHandle);

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: A guard is created only after this thread acquires the mutex.
        let _ = unsafe { ReleaseMutex(HANDLE(self.0.as_raw_handle())) };
    }
}

pub(super) fn acquire() -> io::Result<Guard> {
    let (sid, _) = crate::activation::identity()?;
    named(&format!(r"Global\Strata.FileManager.{sid}"), 1000)
}

pub(super) fn launch(scope: &str) -> io::Result<Guard> {
    use std::hash::{Hash, Hasher};
    let (sid, session) = crate::activation::identity()?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    scope.hash(&mut hash);
    named(
        &format!(
            r"Local\Strata.FileManager.Launch.{sid}.{session}.{:x}",
            hash.finish()
        ),
        10_000,
    )
}

fn named(value: &str, timeout: u32) -> io::Result<Guard> {
    let name = super::registry::wide(std::ffi::OsStr::new(value))?;
    // SAFETY: The name is terminated. The mutex receives the process token's
    // default security descriptor and is scoped by the current user's SID.
    let handle =
        unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.map_err(io::Error::other)?;
    // SAFETY: CreateMutexW returned a newly owned, valid kernel handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    // SAFETY: The owned mutex remains live throughout the bounded wait.
    let outcome = unsafe { WaitForSingleObject(HANDLE(handle.as_raw_handle()), timeout) };
    if outcome != WAIT_OBJECT_0 && outcome != WAIT_ABANDONED {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Another file-manager registration is in progress",
        ));
    }
    Ok(Guard(handle))
}
