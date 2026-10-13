// SPDX-License-Identifier: MIT

//! Native Windows state and config persistence.
//! Linux's owner-aware Unix symlink/permission rules do not apply to Win32.
//! Keep the API shape while delegating actual writes to the tested native
//! backend. Windows config symlinks are intentionally not followed.
use std::{
    io,
    path::{Path, PathBuf},
};

pub(crate) fn config_directory() -> PathBuf {
    gtk::glib::user_config_dir().join("strata")
}

pub(crate) fn state_directory() -> PathBuf {
    gtk::glib::user_state_dir().join("strata")
}

pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    strata_native_io::windows_storage::atomic_write(path, contents)
}

pub(crate) fn atomic_write_config(path: &Path, contents: &[u8]) -> io::Result<()> {
    strata_native_io::windows_storage::atomic_write_config(path, contents)
}
