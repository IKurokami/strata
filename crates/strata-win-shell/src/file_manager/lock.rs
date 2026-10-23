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
    let name = super::registry::wide(std::ffi::OsStr::new(&format!(
        r"Global\Strata.FileManager.{sid}"
    )))?;
    // SAFETY: The name is terminated. The mutex receives the process token's
    // default security descriptor and is scoped by the current user's SID.
    let handle =
        unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) }.map_err(io::Error::other)?;
    // SAFETY: CreateMutexW returned a newly owned, valid kernel handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    // SAFETY: The owned mutex remains live throughout the bounded wait.
    let outcome = unsafe { WaitForSingleObject(HANDLE(handle.as_raw_handle()), 1000) };
    if outcome != WAIT_OBJECT_0 && outcome != WAIT_ABANDONED {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "Another file-manager registration is in progress",
        ));
    }
    Ok(Guard(handle))
}
