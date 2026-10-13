// SPDX-License-Identifier: MIT

//! Native Windows update safety boundary.
//!
//! Linux updates replace an executing binary with rename, examine /proc,
//! signal old processes and run pacman/AUR commands. None of those behaviors
//! may be used on Windows without a signed installer/helper and process-exit
//! verification. Do not download an archive that cannot safely be installed.
//! Merely constructing this service has no I/O, shell invocation or UAC.
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Receiver},
    },
};

use super::release_channel::Version;

const INSTALL_OPEN: u8 = 0;
const INSTALL_CANCELLED: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateMethod {
    InPlace,
    Aur,
    Omarchy,
    Pacman,
}

impl UpdateMethod {
    pub fn is_package_managed(self) -> bool {
        self != Self::InPlace
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateInstall {
    Downloading { downloaded: u64, total: Option<u64> },
    Verifying,
    Installing,
    Finalizing,
    Installed,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub struct InstallCancel(Arc<AtomicU8>);

impl InstallCancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(
                INSTALL_OPEN,
                INSTALL_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
            || self.is_cancelled()
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == INSTALL_CANCELLED
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallRequest {
    pub tag: String,
    pub asset_name: String,
    pub advertised_url: String,
}

/// Keep release checks usable. This is NOT permission to update a running EXE:
/// install_update always refuses until a native signed updater is implemented.
pub fn update_method() -> UpdateMethod {
    UpdateMethod::InPlace
}

pub fn install_update(_request: InstallRequest, cancel: InstallCancel) -> Receiver<UpdateInstall> {
    let (sender, receiver) = mpsc::channel();
    let outcome = if cancel.is_cancelled() {
        UpdateInstall::Cancelled
    } else {
        UpdateInstall::Failed(
            "Automatic installation on Windows is unavailable until a verified native updater is implemented.".to_owned(),
        )
    };
    let _ = sender.send(outcome);
    receiver
}

/// The Linux rollback mechanism is intentionally never run on Windows.
pub(crate) fn rollback_path(directory: &Path) -> PathBuf {
    directory.join(".strata-update-rollback")
}

// Pacman, AUR and Omarchy are Linux-only. Returning a concrete failure here
// prevents an accidental network/package probe in the background updater.
pub(super) fn package_repository_version() -> Result<Version, String> {
    Err("Linux package repositories are unavailable on Windows".to_owned())
}

pub(super) fn aur_repository_version() -> Result<Version, String> {
    Err("AUR updates are unavailable on Windows".to_owned())
}

pub(super) fn omarchy_repository_version() -> Result<Version, String> {
    Err("Omarchy updates are unavailable on Windows".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> InstallRequest {
        InstallRequest {
            tag: "v1.0.0".to_owned(),
            asset_name: "untrusted.tar.gz".to_owned(),
            advertised_url: "https://example.invalid/binary".to_owned(),
        }
    }

    #[test]
    fn cannot_install_or_launch_untrusted_archive() {
        let update = install_update(sample(), InstallCancel::new());
        let event = update.recv().unwrap();
        assert!(matches!(event, UpdateInstall::Failed(message) if message.contains("Windows")));
        assert!(update.try_recv().is_err());
    }

    #[test]
    fn cancellation_precedes_any_installation() {
        let cancel = InstallCancel::new();
        assert!(cancel.cancel());
        assert!(cancel.cancel());
        let update = install_update(sample(), cancel);
        assert_eq!(update.recv().unwrap(), UpdateInstall::Cancelled);
    }
}
