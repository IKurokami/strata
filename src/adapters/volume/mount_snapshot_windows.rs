// SPDX-License-Identifier: MIT

//! No synchronous GIO/Win32 volume enumeration from Windows navigation.
//! A future Windows volume worker should publish immutable snapshots to GTK.
use std::{cell::RefCell, path::PathBuf};

use super::MountTable;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemovableRoot {
    pub(crate) uuid: String,
    pub(crate) path: PathBuf,
}

pub(crate) struct MountSnapshot {
    pub(crate) table: MountTable,
    pub(crate) removable: Vec<RemovableRoot>,
}

thread_local! {
    static MOUNTS: RefCell<Option<MountSnapshot>> = const { RefCell::new(None) };
}

pub(crate) fn with_mount_snapshot<T>(read: impl FnOnce(&MountSnapshot) -> T) -> T {
    MOUNTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        read(cache.get_or_insert_with(|| {
            let removable = strata_win_shell::enumerate_volumes()
                .into_iter()
                .filter(|v| v.drive_type.is_removable())
                .map(|v| RemovableRoot {
                    uuid: v
                        .serial_number
                        .map(|s| format!("{:08X}", s))
                        .unwrap_or_else(|| "00000000".to_string()),
                    path: v.root,
                })
                .collect();
            MountSnapshot {
                table: MountTable::current(),
                removable,
            }
        }))
    })
}

pub(crate) fn invalidate_mount_snapshot() {
    MOUNTS.with(|cache| {
        if let Ok(mut cache) = cache.try_borrow_mut() {
            *cache = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_volumes_finds_c_drive() {
        let volumes = strata_win_shell::enumerate_volumes();
        assert!(!volumes.is_empty());
        let c_drive = volumes.iter().find(|v| v.root.to_string_lossy().starts_with("C:"));
        assert!(c_drive.is_some());
    }

    #[test]
    fn test_path_to_dos_device() {
        let wide = strata_win_shell::path_to_dos_device(std::path::Path::new("C:\\Windows")).unwrap();
        let s = String::from_utf16_lossy(&wide);
        assert_eq!(s, "\\\\.\\C:\0");
        assert!(strata_win_shell::path_to_dos_device(std::path::Path::new("\\\\server\\share")).is_err());
    }
}
