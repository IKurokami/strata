// SPDX-License-Identifier: MIT

//! Native Windows state and config persistence.
//! Linux's owner-aware Unix symlink/permission rules do not apply to Win32.
//! Keep the API shape while delegating actual writes to the tested native
//! backend. Windows config symlinks are intentionally not followed.
use std::{
    io,
    path::{Path, PathBuf},
};

/// Preserve the localized destination-refusal error contract used by the
/// theme editor and generic I/O error presenter.
#[derive(Debug)]
pub(crate) struct DestinationError {
    path: PathBuf,
}

impl std::fmt::Display for DestinationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "refusing to replace non-regular destination {}",
            self.path.display()
        )
    }
}

impl std::error::Error for DestinationError {}

impl DestinationError {
    pub(crate) fn message(&self) -> String {
        rust_i18n::t!(
            "The destination “%{path}” is not a regular file",
            path = self.path.display()
        )
        .into_owned()
    }
}

fn translate_error(path: &Path, error: io::Error) -> io::Error {
    if error
        .get_ref()
        .is_some_and(|inner| inner.is::<strata_native_io::windows_storage::DestinationError>())
    {
        io::Error::new(
            error.kind(),
            DestinationError {
                path: path.to_path_buf(),
            },
        )
    } else {
        error
    }
}

pub(crate) fn config_directory() -> PathBuf {
    gtk::glib::user_config_dir().join("strata")
}

pub(crate) fn state_directory() -> PathBuf {
    gtk::glib::user_state_dir().join("strata")
}

pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    strata_native_io::windows_storage::atomic_write(path, contents)
        .map_err(|error| translate_error(path, error))
}

pub(crate) fn atomic_write_config(path: &Path, contents: &[u8]) -> io::Result<()> {
    strata_native_io::windows_storage::atomic_write_config(path, contents)
        .map_err(|error| translate_error(path, error))
}
