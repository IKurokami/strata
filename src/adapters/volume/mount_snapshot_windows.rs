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
        read(cache.get_or_insert_with(|| MountSnapshot {
            table: MountTable::current(),
            // Never misreport a local drive as removable based on its letter.
            // Native drive enumeration is pending and will run off the UI thread.
            removable: Vec::new(),
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
