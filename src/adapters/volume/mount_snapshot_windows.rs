// SPDX-License-Identifier: MIT

//! No synchronous GIO/Win32 volume enumeration from Windows navigation.
//! The cached snapshot is refreshed by a dedicated worker and read by GTK.
use std::{
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

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

static MOUNTS: OnceLock<Mutex<MountSnapshot>> = OnceLock::new();
static ENUMERATION_STARTED: AtomicBool = AtomicBool::new(false);

fn mount_cache() -> &'static Mutex<MountSnapshot> {
    MOUNTS.get_or_init(|| {
        Mutex::new(MountSnapshot {
            table: MountTable::current(),
            removable: Vec::new(),
        })
    })
}

fn refresh_in_background() {
    if ENUMERATION_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    thread::Builder::new()
        .name("strata-windows-volume-snapshot".into())
        .spawn(|| {
            let removable = strata_win_shell::enumerate_volumes()
                .into_iter()
                .filter(|volume| volume.drive_type.is_removable())
                .map(|volume| RemovableRoot {
                    uuid: volume
                        .serial_number
                        .map(|serial| format!("{serial:08X}"))
                        .unwrap_or_else(|| "00000000".to_owned()),
                    path: volume.root,
                })
                .collect();
            if let Ok(mut snapshot) = mount_cache().lock() {
                snapshot.removable = removable;
            }
        })
        .ok();
}

pub(crate) fn with_mount_snapshot<T>(read: impl FnOnce(&MountSnapshot) -> T) -> T {
    refresh_in_background();
    let snapshot = mount_cache().lock().expect("Windows mount snapshot");
    read(&snapshot)
}

pub(crate) fn invalidate_mount_snapshot() {
    ENUMERATION_STARTED.store(false, Ordering::Release);
    if let Ok(mut snapshot) = mount_cache().try_lock() {
        snapshot.removable.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn test_enumerate_volumes_finds_c_drive() {
        let volumes = strata_win_shell::enumerate_volumes();
        assert!(!volumes.is_empty());
        let c_drive = volumes
            .iter()
            .find(|v| v.root.to_string_lossy().starts_with("C:"));
        assert!(c_drive.is_some());
    }

    #[test]
    fn test_path_to_dos_device() {
        let wide =
            strata_win_shell::path_to_dos_device(std::path::Path::new("C:\\Windows")).unwrap();
        let s = String::from_utf16_lossy(&wide);
        assert_eq!(s, "\\\\.\\C:\0");
        assert!(
            strata_win_shell::path_to_dos_device(std::path::Path::new("\\\\server\\share"))
                .is_err()
        );
    }
}
