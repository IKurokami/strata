//! Native file activation, called only by the standalone shell helper.

use std::{io, os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::{
            Shell::{
                OAIF_EXEC, OPENASINFO, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
                SHOpenWithDialog, ShellExecuteExW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{Error, PCWSTR},
};

fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if !path.is_absolute() || wide.contains(&0) {
        return Err(io::Error::from_raw_os_error(87));
    }
    wide.push(0);
    Ok(wide)
}

fn os_error(error: Error) -> io::Error {
    let code = error.code().0 as u32;
    if code & 0xffff_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((code & 0xffff) as i32)
    } else {
        io::Error::other(error.to_string())
    }
}

/// Resolve the associated executable without running a shell command or loading
/// an application's extension into the caller. Invoke from a background worker.
pub fn associated_executable(path: &Path) -> io::Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::{
        Win32::UI::Shell::{ASSOCF_NONE, ASSOCSTR_EXECUTABLE, AssocQueryStringW},
        core::PWSTR,
    };
    let extension = path
        .extension()
        .ok_or_else(|| io::Error::from_raw_os_error(1155))?;
    let association: Vec<_> = std::ffi::OsString::from(format!(".{}", extension.to_string_lossy()))
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut length = 0;
    // SAFETY: Valid terminated association string and writable length, no output buffer.
    let _ = unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_EXECUTABLE,
            PCWSTR(association.as_ptr()),
            PCWSTR::null(),
            None,
            &mut length,
        )
    };
    if length == 0 || length > 32768 {
        return Err(io::Error::from_raw_os_error(1155));
    }
    let mut output = vec![0u16; length as usize];
    // SAFETY: The output owns length UTF-16 code units for the synchronous call.
    unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_EXECUTABLE,
            PCWSTR(association.as_ptr()),
            PCWSTR::null(),
            Some(PWSTR(output.as_mut_ptr())),
            &mut length,
        )
    }
    .ok()
    .map_err(os_error)?;
    let end = output
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(output.len());
    let executable = std::path::PathBuf::from(std::ffi::OsString::from_wide(&output[..end]));
    if !executable.is_absolute() {
        return Err(io::Error::from_raw_os_error(1155));
    }
    Ok(executable)
}

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: Constructed only after successful initialization on this thread.
        unsafe { CoUninitialize() };
    }
}

/// Open a file with its Windows default verb, or show the native application
/// chooser. Shell extensions stay out of the GTK process. UAC remains Windows'
/// responsibility; cancelling either native prompt is a normal outcome.
pub fn open(path: &Path, choose: bool) -> io::Result<()> {
    let file = wide_path(path)?;
    let directory = path.parent().map(wide_path).transpose()?;
    // SAFETY: The standalone helper owns this apartment until the call completes.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(os_error)?;
    let _apartment = Apartment;
    let result = if choose {
        choose_application(&file)
    } else {
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            // The helper has no message loop; complete DDE before it exits.
            fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpFile: PCWSTR(file.as_ptr()),
            lpDirectory: directory
                .as_ref()
                .map_or(PCWSTR::null(), |value| PCWSTR(value.as_ptr())),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: All UTF-16 buffers outlive the synchronous shell operation.
        let result = unsafe { ShellExecuteExW(&mut info) }.map_err(os_error);
        match result {
            Err(error) if error.raw_os_error() == Some(1155) => choose_application(&file),
            result => result,
        }
    };
    match result {
        Err(error) if error.raw_os_error() == Some(1223) => Ok(()),
        result => result,
    }
}

fn choose_application(file: &[u16]) -> io::Result<()> {
    let info = OPENASINFO {
        pcszFile: PCWSTR(file.as_ptr()),
        oaifInFlags: OAIF_EXEC,
        ..Default::default()
    };
    // SAFETY: The path is NUL-terminated and remains live until the dialog closes.
    unsafe { SHOpenWithDialog(None, &info) }.map_err(os_error)
}

#[cfg(test)]
mod tests;
