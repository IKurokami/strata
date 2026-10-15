// SPDX-License-Identifier: MIT

//! Windows native volume discovery, flush, and ejection.
//!
//! All Win32 handle creation, DeviceIoControl, and volume enumeration stay inside
//! this crate to ensure the root Strata crate remains pure safe Rust.

use std::{
    io,
    path::{Path, PathBuf},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, FlushFileBuffers, GetDriveTypeW,
            GetLogicalDrives, GetVolumeInformationW, OPEN_EXISTING,
        },
        System::Ioctl::{
            FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, FSCTL_UNLOCK_VOLUME,
            IOCTL_STORAGE_EJECT_MEDIA,
        },
    },
    core::PCWSTR,
};

/// Type of volume classified by the Win32 subsystem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriveType {
    Unknown,
    NoRootDir,
    Removable,
    Fixed,
    Remote,
    CdRom,
    RamDisk,
}

impl DriveType {
    pub fn is_removable(self) -> bool {
        matches!(self, Self::Removable | Self::CdRom)
    }
}

/// Metadata and identification facts for a mounted Windows volume.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeInfo {
    pub root: PathBuf,
    pub drive_type: DriveType,
    pub label: Option<String>,
    pub fs_type: Option<String>,
    pub serial_number: Option<u32>,
}

/// Enumerates all mounted logical drives on Windows without blocking on offline network shares.
pub fn enumerate_volumes() -> Vec<VolumeInfo> {
    // SAFETY: GetLogicalDrives takes no pointers and has no side effects.
    let mask = unsafe { GetLogicalDrives() };
    let mut volumes = Vec::new();

    for i in 0..26 {
        if (mask & (1 << i)) != 0 {
            let letter = (b'A' + i) as char;
            let root_str = format!("{}:\\", letter);
            let root_wide: Vec<u16> = root_str.encode_utf16().chain(Some(0)).collect();

            // SAFETY: root_wide is a valid null-terminated UTF-16 string.
            let drive_type_raw = unsafe { GetDriveTypeW(PCWSTR(root_wide.as_ptr())) };
            let drive_type = match drive_type_raw {
                2 => DriveType::Removable,
                3 => DriveType::Fixed,
                4 => DriveType::Remote,
                5 => DriveType::CdRom,
                6 => DriveType::RamDisk,
                1 => DriveType::NoRootDir,
                _ => DriveType::Unknown,
            };

            // Skip network drives for volume info to prevent UI hangs if offline
            let (label, fs_type, serial_number) = if drive_type != DriveType::Remote {
                get_volume_info(&root_wide)
            } else {
                (None, None, None)
            };

            volumes.push(VolumeInfo {
                root: PathBuf::from(root_str),
                drive_type,
                label,
                fs_type,
                serial_number,
            });
        }
    }

    volumes
}

fn get_volume_info(root_wide: &[u16]) -> (Option<String>, Option<String>, Option<u32>) {
    let mut name_buf = [0u16; 261];
    let mut fs_buf = [0u16; 261];
    let mut serial: u32 = 0;

    // SAFETY: buffers are sized to MAX_PATH + 1, root_wide is null-terminated.
    let success = unsafe {
        GetVolumeInformationW(
            PCWSTR(root_wide.as_ptr()),
            Some(&mut name_buf),
            Some(&mut serial),
            None,
            None,
            Some(&mut fs_buf),
        )
    };

    if success.is_ok() {
        let label = null_term_to_string(&name_buf);
        let fs = null_term_to_string(&fs_buf);
        (
            if label.is_empty() { None } else { Some(label) },
            if fs.is_empty() { None } else { Some(fs) },
            Some(serial),
        )
    } else {
        (None, None, None)
    }
}

fn null_term_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

pub fn path_to_dos_device(path: &Path) -> io::Result<Vec<u16>> {
    let path_str = path.to_string_lossy();
    let bytes = path_str.as_bytes();

    let letter = if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        (bytes[0] as char).to_ascii_uppercase()
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Path does not contain a valid Windows drive letter: {}",
                path.display()
            ),
        ));
    };

    let dev_str = format!("\\\\.\\{}:", letter);
    let dev_wide: Vec<u16> = dev_str.encode_utf16().chain(Some(0)).collect();
    Ok(dev_wide)
}

struct AutoHandle(HANDLE);

impl Drop for AutoHandle {
    fn drop(&mut self) {
        if self.0 != HANDLE::default() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: self.0 is a valid open handle.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Flushes filesystem buffers on the volume associated with `path`.
pub fn flush_volume(path: &Path) -> io::Result<()> {
    let dev_wide = path_to_dos_device(path)?;

    // SAFETY: CreateFileW opens the volume device object.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(dev_wide.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };

    let handle = match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => AutoHandle(h),
        _ => return Err(io::Error::last_os_error()),
    };

    // SAFETY: handle is valid.
    let ok = unsafe { FlushFileBuffers(handle.0) };
    if ok.is_ok() {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Dismounts and ejects the removable media for `path`.
pub fn eject_volume(path: &Path) -> io::Result<()> {
    let dev_wide = path_to_dos_device(path)?;

    // First flush
    let _ = flush_volume(path);

    // SAFETY: Open volume for locking and dismounting
    let handle = unsafe {
        CreateFileW(
            PCWSTR(dev_wide.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };

    let handle = match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => AutoHandle(h),
        _ => return Err(io::Error::last_os_error()),
    };

    let mut bytes_returned = 0u32;

    // 1. Lock Volume
    // SAFETY: DeviceIoControl called on open handle.
    unsafe {
        let _ = windows::Win32::System::IO::DeviceIoControl(
            handle.0,
            FSCTL_LOCK_VOLUME,
            None,
            0,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        );
    }

    // 2. Dismount Volume
    // SAFETY: DeviceIoControl called on open handle.
    let dismounted = unsafe {
        windows::Win32::System::IO::DeviceIoControl(
            handle.0,
            FSCTL_DISMOUNT_VOLUME,
            None,
            0,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        )
    };

    // 3. Eject Media
    // SAFETY: DeviceIoControl called on open handle.
    let ejected = unsafe {
        windows::Win32::System::IO::DeviceIoControl(
            handle.0,
            IOCTL_STORAGE_EJECT_MEDIA,
            None,
            0,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        )
    };

    // 4. Unlock Volume if ejection didn't detach
    // SAFETY: DeviceIoControl called on open handle.
    unsafe {
        let _ = windows::Win32::System::IO::DeviceIoControl(
            handle.0,
            FSCTL_UNLOCK_VOLUME,
            None,
            0,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        );
    }

    if ejected.is_ok() || dismounted.is_ok() {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_volumes_finds_c_drive() {
        let volumes = enumerate_volumes();
        assert!(!volumes.is_empty(), "Must find at least one drive");
        let c = volumes.iter().find(|v| v.root == Path::new("C:\\"));
        assert!(c.is_some(), "C: drive must exist on Windows");
        let c = c.unwrap();
        assert_eq!(c.drive_type, DriveType::Fixed);
        assert!(c.fs_type.is_some());
    }

    #[test]
    fn test_path_to_dos_device() {
        let wide = path_to_dos_device(Path::new("C:\\Windows")).unwrap();
        let s = String::from_utf16_lossy(&wide);
        assert_eq!(s, "\\\\.\\C:\0");

        assert!(path_to_dos_device(Path::new("\\\\server\\share")).is_err());
    }
}
