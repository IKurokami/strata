// SPDX-License-Identifier: MIT

pub(crate) mod bookmarks;
pub(crate) mod directory_summary;
mod file_manager1;
mod gio_location;
mod local_actions;
mod local_files;
#[cfg(not(windows))]
mod local_jobs;
#[cfg(windows)]
#[path = "local_jobs_windows.rs"]
mod local_jobs;
#[cfg(not(windows))]
mod local_operations;
#[cfg(windows)]
#[path = "local_operations_windows.rs"]
mod local_operations;
mod local_preview;
#[cfg(windows)]
pub(crate) mod recycle_bin;
pub(crate) mod trash;
#[cfg(not(windows))]
pub(crate) mod trash_restore;
#[cfg(windows)]
#[path = "trash_restore_windows.rs"]
pub(crate) mod trash_restore;
mod volume;

pub(crate) use file_manager1::{RevealRequest, export_file_manager};
pub(crate) use gio_location::{
    gio_file_for_location, location_for_file, read_file_for_location, reveal_target_for_file,
};
pub(crate) use local_actions::{LocalActionStore, resolve_executable as resolve_action_executable};
pub use local_files::LocalFileSource;
pub(crate) use local_files::query_file_entry;
pub(crate) use local_files::{RecentRemovalState, recent_remove_entries};
pub(crate) use local_jobs::LocalActionRunner;
pub use local_operations::LocalOperationProvider;
pub(crate) use local_operations::{
    ARCHIVE_PREVIEW_FAILED_MESSAGE, ARCHIVE_TOO_LARGE_MESSAGE, ARCHIVE_UNSUPPORTED_MESSAGE,
    INVALID_ARCHIVE, MAX_ARCHIVE_PASSWORD_BYTES, MAYBE_BAD_PASSWORD, PASSWORD_REQUIRED,
    archive_payload_valid, encode_archive_result, flush_filesystem, list_archive_entries_direct,
};
#[cfg(all(test, windows))]
pub(crate) use local_operations::{ArchiveListing, ArchiveListingStatus, decode_archive_listing};
#[cfg(all(test, not(windows)))]
pub(crate) use local_operations::{
    ArchiveListing, ArchiveListingStatus, decode_archive_listing, write_compression_fixture,
};
#[cfg(all(test, not(windows)))]
pub(crate) use local_operations::{
    clear_sync_probe, install_sync_probe, release_sync_probe, sync_probe_is_blocked,
    sync_probe_len, sync_probe_observations,
};
pub use local_preview::LocalPreviewProvider;
pub(crate) use local_preview::clear_preview_cache;
pub(crate) use volume::{DropVolumeQuery, DropVolumes, lookup_drop_volumes};
pub(crate) use volume::{
    MountTable, RemovableRoot, invalidate_mount_snapshot, mount_can_unplug, mount_is_removable,
    volume_can_unplug, with_mount_snapshot,
};
