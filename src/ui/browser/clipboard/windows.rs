// SPDX-License-Identifier: MIT

use std::os::windows::ffi::OsStrExt;

use gtk::{gio, prelude::*};

pub(super) const FILE_DROP_MIME: &str = "application/x.windows.CF_HDROP";

pub(super) fn file_drop_data(files: &[gio::File]) -> Option<Vec<u8>> {
    if files.is_empty() {
        return None;
    }
    // DROPFILES has five 32-bit fields on both Win32 and Win64. The trailing
    // names are absolute UTF-16 paths, individually terminated, then an empty name.
    let mut data: Vec<_> = [20_u32, 0, 0, 0, 1]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    for file in files {
        let path = file.path()?;
        if !path.is_absolute() {
            return None;
        }
        for unit in path.as_os_str().encode_wide() {
            if unit == 0 {
                return None;
            }
            data.extend_from_slice(&unit.to_le_bytes());
        }
        data.extend_from_slice(&0_u16.to_le_bytes());
    }
    data.extend_from_slice(&0_u16.to_le_bytes());
    Some(data)
}

#[cfg(test)]
mod tests;
