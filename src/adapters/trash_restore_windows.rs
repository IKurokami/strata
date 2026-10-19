// SPDX-License-Identifier: MIT

//! Windows Recycle Bin restore must use shell item identity and verified
//! per-volume provenance. Linux freedesktop.org .trashinfo, uid/inode,
//! bind-mount checks and Unix symlink rules are NOT Windows restore rules.
//! Resolve destinations from the verified original path; the actual move is
//! performed later through the native Shell Recycle Bin API.
use std::path::{Path, PathBuf};

use crate::model::Location;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RestoreTargetError {
    reason: &'static str,
}
impl RestoreTargetError {
    fn unsupported() -> Self {
        Self {
            reason: "The original location is unavailable",
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
    source_path: &Path,
    orig_path: &Path,
    trash_root: &Path,
    _trash_info: Option<PathBuf>,
    _context: &RestoreContext,
) -> Result<RestorePlan, RestoreTargetError> {
    if !orig_path.is_absolute()
        || !source_path.is_absolute()
        || source_path.starts_with(trash_root)
        || orig_path.parent().is_none_or(|parent| !parent.is_dir())
        || orig_path.symlink_metadata().is_ok()
    {
        return Err(RestoreTargetError::unsupported());
    }
    Ok(RestorePlan {
        source_path: source_path.to_path_buf(),
        destination: orig_path.to_path_buf(),
        allowed_root: orig_path.parent().unwrap_or(orig_path).to_path_buf(),
        trash_info: None,
    })
}
pub(crate) async fn plan_restore_for_location(
    source: &Location,
    original_target: Option<&Location>,
    _trash_info: Option<&Path>,
    physical_path: Option<&Path>,
    context: &RestoreContext,
) -> Result<RestorePlan, RestoreTargetError> {
    if super::recycle_bin::is_item(source)
        && let Some(recycled) = super::recycle_bin::path(source)
    {
        let destination =
            gtk::gio::spawn_blocking(move || strata_win_shell::original_path(&recycled))
                .await
                .map_err(|_| RestoreTargetError::unsupported())?
                .map_err(|_| RestoreTargetError::unsupported())?;
        return plan_restore_from_known_paths(
            &destination,
            &destination,
            Path::new("\\\\?\\"),
            None,
            context,
        );
    }
    let destination = physical_path
        .or_else(|| original_target.and_then(Location::native_path))
        .ok_or_else(RestoreTargetError::unsupported)?;
    // A `trash:///` Location has no native path on Windows. The Shell API
    // later resolves that virtual item by its original path; retain the
    // destination as the logical source for the confirmation plan instead of
    // pretending the destination itself is a filesystem source.
    let source_path = source
        .native_path()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| destination.to_path_buf());
    plan_restore_from_known_paths(
        &source_path,
        destination,
        Path::new("\\\\?\\"),
        None,
        context,
    )
}
pub(crate) async fn restore_destinations_for_locations(
    items: Vec<(Location, Option<PathBuf>)>,
) -> Vec<Result<PathBuf, RestoreTargetError>> {
    let mut results = Vec::with_capacity(items.len());
    for (source, physical_path) in items {
        let result = match physical_path {
            Some(destination) => plan_restore_for_location(
                &source,
                None,
                None,
                Some(&destination),
                &RestoreContext::current(),
            )
            .await
            .map(|plan| plan.destination),
            None => Err(RestoreTargetError::unsupported()),
        };
        results.push(result);
    }
    results
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
        assert!(
            plan_restore_from_known_paths(
                Path::new("C:\\$Recycle.Bin\\file"),
                Path::new("C:\\test"),
                Path::new("C:\\$Recycle.Bin"),
                None,
                &RestoreContext::current(),
            )
            .is_err()
        );
    }

    #[test]
    fn plans_a_restore_from_the_verified_original_path() {
        let root = tempfile::tempdir().expect("temporary root");
        let source = root.path().join("recycled-item");
        let destination = root.path().join("restored.txt");
        std::fs::write(&source, b"recycled").expect("source");
        let plan = plan_restore_from_known_paths(
            &source,
            &destination,
            root.path(),
            None,
            &RestoreContext::current(),
        )
        .expect("restore plan");
        assert_eq!(plan.source_path, source);
        assert_eq!(plan.destination, destination);
    }
}
