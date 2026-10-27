// SPDX-License-Identifier: MIT

use std::io;
use windows::{
    Win32::{
        Foundation::E_ABORT,
        UI::Shell::{
            COPYENGINE_E_ACCESS_DENIED_DEST, COPYENGINE_E_ACCESS_DENIED_SRC,
            COPYENGINE_E_DISK_FULL, COPYENGINE_E_DISK_FULL_CLEAN,
            COPYENGINE_E_SHARING_VIOLATION_DEST, COPYENGINE_E_SHARING_VIOLATION_SRC,
            COPYENGINE_E_USER_CANCELLED,
        },
    },
    core::{Error, HRESULT},
};

pub(super) fn error(code: HRESULT) -> io::Error {
    let kind = match code {
        E_ABORT | COPYENGINE_E_USER_CANCELLED => io::ErrorKind::Interrupted,
        COPYENGINE_E_ACCESS_DENIED_SRC | COPYENGINE_E_ACCESS_DENIED_DEST => {
            io::ErrorKind::PermissionDenied
        }
        COPYENGINE_E_SHARING_VIOLATION_SRC | COPYENGINE_E_SHARING_VIOLATION_DEST => {
            io::ErrorKind::ResourceBusy
        }
        COPYENGINE_E_DISK_FULL | COPYENGINE_E_DISK_FULL_CLEAN => io::ErrorKind::StorageFull,
        _ if code.0 as u32 & 0xffff_0000 == 0x8007_0000 => {
            let win32 = code.0 & 0xffff;
            match win32 {
                995 | 1223 => io::ErrorKind::Interrupted,
                32 | 33 => io::ErrorKind::ResourceBusy,
                _ => io::Error::from_raw_os_error(win32).kind(),
            }
        }
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, Error::from_hresult(code))
}

pub(super) fn resolve(
    performed: windows::core::Result<()>,
    aborted: windows::core::Result<bool>,
    failure: Option<HRESULT>,
) -> io::Result<()> {
    // The Shell also sets its abort flag after failures such as sharing violations.
    // Preserve the per-item cause instead of replacing it with E_ABORT.
    if let Some(code) = failure {
        if error(code).kind() == io::ErrorKind::Interrupted
            && let Err(ref performed_error) = performed
            && error(performed_error.code()).kind() != io::ErrorKind::Interrupted
        {
            return Err(error(performed_error.code()));
        }
        return Err(error(code));
    }
    performed.map_err(|failure| error(failure.code()))?;
    if aborted.map_err(|failure| error(failure.code()))? {
        return Err(error(E_ABORT));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
