// SPDX-License-Identifier: MIT

//! Zero-I/O Windows mount facts. Volume enumeration and mapped-drive detection
//! require a separate, asynchronous native adapter; never call Win32 volume
//! discovery or shell extensions from the folder-navigation hot path.
use std::path::Path;

use strata_native_io::windows_paths::{PathKind, classify};

#[derive(Clone, Default)]
pub(crate) struct MountTable;

impl MountTable {
    pub(crate) fn current() -> Self {
        Self
    }

    pub(crate) fn fs_type_for(&self, _path: &Path) -> Option<&str> {
        // Do not infer a filesystem type from a drive letter or extension.
        None
    }

    pub(super) fn is_remote_path(&self, path: &Path) -> bool {
        matches!(classify(path), PathKind::NetworkAbsolute)
    }

    pub(crate) fn is_network_path(&self, path: &Path) -> bool {
        self.is_remote_path(path)
    }

    pub(crate) fn has_mount_inside(&self, _directory: &Path) -> bool {
        false
    }

    pub(super) fn query_may_block(&self, path: &Path) -> bool {
        // UNC can block, so existing volume probes must remain asynchronous.
        // Mapped remote drives are not yet identified and remain a blocker.
        self.is_remote_path(path)
    }

    pub(crate) fn is_mount_point(&self, path: &Path) -> bool {
        matches!(
            classify(path),
            PathKind::LocalAbsolute | PathKind::NetworkAbsolute
        ) && path.parent().is_none()
    }

    pub(crate) fn mount_point_for(&self, _path: &Path) -> Option<&Path> {
        // Never claim an unknown drive's mount root or volume identity.
        None
    }

    pub(crate) fn trash_scan_mounts(&self) -> impl Iterator<Item = &Path> {
        // Do not reuse Linux's filesystem trash search for the Windows Recycle Bin.
        std::iter::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_unc_without_probing_and_does_not_assume_mapped_drive() {
        let mounts = MountTable::current();
        assert!(mounts.query_may_block(Path::new(r"\\server\share\folder")));
        assert!(mounts.is_network_path(Path::new(r"\\server\share\item")));
        assert!(!mounts.is_network_path(Path::new(r"C:\Users\Public")));
        assert!(mounts.fs_type_for(Path::new(r"C:\")).is_none());
    }
}
