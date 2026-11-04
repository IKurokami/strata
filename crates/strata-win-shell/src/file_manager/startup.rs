//! Current-user application startup, independent of Explorer replacement.

use super::{
    registry::{self, Value},
    same_path,
};
use std::{
    ffi::OsString,
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const NAME: &str = "Strata";

pub fn set_enabled(executable: &Path, enabled: bool) -> io::Result<()> {
    apply(RUN, executable, enabled)
}

fn registered_executable(value: &Value) -> Option<PathBuf> {
    let command = value.os_string()?;
    let wide = command.encode_wide().collect::<Vec<_>>();
    let suffix = " --startup".encode_utf16().collect::<Vec<_>>();
    let wide = wide.strip_suffix(suffix.as_slice()).unwrap_or(&wide);
    if wide.first() != Some(&(b'"' as u16)) || wide.last() != Some(&(b'"' as u16)) || wide.len() < 3
    {
        return None;
    }
    let path = PathBuf::from(OsString::from_wide(&wide[1..wide.len() - 1]));
    (path.is_absolute() && !wide[1..wide.len() - 1].contains(&(b'"' as u16))).then_some(path)
}

fn apply(key: &str, executable: &Path, enabled: bool) -> io::Result<()> {
    if !executable.is_absolute() {
        return Err(io::Error::from_raw_os_error(87));
    }
    let current = registry::get(key, NAME)?;
    let owner = current.as_ref().and_then(registered_executable);
    if !enabled {
        if owner
            .as_deref()
            .is_some_and(|owner| same_path(owner, executable))
        {
            registry::set(key, NAME, None)?;
        }
        return Ok(());
    }
    // Another Strata installation may move this entry; foreign commands are preserved.
    if current.is_some()
        && !owner.as_deref().is_some_and(|owner| {
            same_path(owner, executable)
                || owner
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("strata.exe"))
        })
    {
        return Err(io::Error::from_raw_os_error(5));
    }
    if !executable.is_file() {
        return Err(io::Error::from_raw_os_error(2));
    }
    let mut command = OsString::from("\"");
    command.push(executable);
    command.push("\" --startup");
    let desired = Value::text(&command)?;
    if current.as_ref() != Some(&desired) {
        registry::set(key, NAME, Some(&desired))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
