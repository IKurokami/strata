// SPDX-License-Identifier: MIT

//! Windows Recycle Bin restore must use shell item identity and verified
//! per-volume provenance. Linux freedesktop.org .trashinfo, uid/inode,
//! bind-mount checks and Unix symlink rules are NOT Windows restore rules.
//! Fail without filesystem I/O until a native Recycle Bin service exists.
use std::path::{Path, PathBuf};

use crate::model::Location;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RestoreTargetError {
    reason: &'static str,
}
impl RestoreTargetError {
    fn unsupported() -> Self {
        Self {
            reason: "Windows Recycle Bin restore is not available yet",
        }
    }
    pub(crate) fn message(&self) -> &str {
        self.reason
    }
    pub(crate) fn user_message(&self) -> String {
        crate::i18n::tr(self.reason)
    }
}
impl std::fmt::Display for RestoreTargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RestorePlan {
    pub(crate) source_path: PathBuf,
    pub(crate) destination: PathBuf,
    pub(crate) allowed_root: PathBuf,
    pub(crate) trash_info: Option<PathBuf>,
}

#[derive(Clone, Default)]
pub(crate) struct RestoreContext;
impl RestoreContext {
    pub(crate) fn current() -> Self {
        Self
    }
}

pub(crate) fn decode_trashinfo_path(_encoded: &str) -> Option<PathBuf> {
    None
}
pub(crate) fn plan_restore_from_known_paths(
    _source_path: &Path,
    _orig_path: &Path,
    _trash_root: &Path,
    _trash_info: Option<PathBuf>,
    _context: &RestoreContext,
) -> Result<RestorePlan, RestoreTargetError> {
    Err(RestoreTargetError::unsupported())
}
pub(crate) async fn plan_restore_for_location(
    _source: &Location,
    _original_target: Option<&Location>,
    _trash_info: Option<&Path>,
    _physical_path: Option<&Path>,
    _context: &RestoreContext,
) -> Result<RestorePlan, RestoreTargetError> {
    Err(RestoreTargetError::unsupported())
}
pub(crate) async fn restore_destinations_for_locations(
    items: Vec<(Location, Option<PathBuf>)>,
) -> Vec<Result<PathBuf, RestoreTargetError>> {
    items
        .into_iter()
        .map(|_| Err(RestoreTargetError::unsupported()))
        .collect()
}
pub(crate) fn trash_root_from_files_path(_path: &Path) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_linux_trash_metadata_in_windows_recycle_bin() {
        assert!(decode_trashinfo_path("file:///C:/private/test").is_none());
        assert!(plan_restore_from_known_paths(
            Path::new("C:\\$Recycle.Bin\\file"),
            Path::new("C:\\test"),
            Path::new("C:\\$Recycle.Bin"),
            None,
            &RestoreContext::current(),
        )
        .is_err());
    }
}
