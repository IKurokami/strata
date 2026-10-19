// SPDX-License-Identifier: MIT

use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, LoadHandle},
};
use gtk::{gio, glib};
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, UNIX_EPOCH},
};

pub(crate) fn is_root(location: &Location) -> bool {
    location.uri_value() == Some("trash:///")
}

pub(crate) fn location(path: &Path) -> Location {
    use std::fmt::Write;
    let mut uri = String::from("trash:///windows-");
    for word in path.as_os_str().encode_wide() {
        write!(uri, "{word:04x}").expect("write to String");
    }
    Location::uri(uri)
}

pub(crate) fn path(location: &Location) -> Option<PathBuf> {
    let encoded = location.uri_value()?.strip_prefix("trash:///windows-")?;
    let mut components = encoded.split('/');
    let mut path = PathBuf::from(decode(components.next()?)?);
    if !path.is_absolute() {
        return None;
    }
    for component in components {
        let name = decode(component)?;
        if !strata_native_io::windows_paths::valid_child_name(&name) {
            return None;
        }
        path.push(name);
    }
    Some(path)
}

pub(crate) fn is_item(location: &Location) -> bool {
    location
        .uri_value()
        .and_then(|uri| uri.strip_prefix("trash:///windows-"))
        .is_some_and(|name| !name.contains('/'))
        && path(location).is_some()
}

fn decode(encoded: &str) -> Option<OsString> {
    if encoded.is_empty() || !encoded.len().is_multiple_of(4) || !encoded.is_ascii() {
        return None;
    }
    let words = encoded
        .as_bytes()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u16::from_str_radix(std::str::from_utf8(word).ok()?, 16).ok())
        .collect::<Option<Vec<_>>>()?;
    if words.contains(&0) {
        return None;
    }
    Some(OsString::from_wide(&words))
}

pub(crate) fn child_location(parent: &Location, name: &std::ffi::OsStr) -> Location {
    use std::fmt::Write;
    let mut uri = parent.uri_value().unwrap_or_default().to_owned();
    uri.push('/');
    for word in name.encode_wide() {
        write!(uri, "{word:04x}").expect("write to String");
    }
    Location::uri(uri)
}

pub(crate) fn verified_root(location: &Location) -> Option<PathBuf> {
    let uri = location.uri_value()?;
    let root = uri.strip_prefix("trash:///windows-")?.split('/').next()?;
    path(&Location::uri(format!("trash:///windows-{root}")))
}

pub(crate) fn error(reason: impl std::fmt::Display) -> glib::Error {
    tracing::warn!(error = %reason, "native Recycle Bin lookup failed");
    glib::Error::new(
        gio::IOErrorEnum::Failed,
        &crate::i18n::tr("Unable to read Trash"),
    )
}

pub(crate) fn operation_error_message(error: &std::io::Error) -> String {
    if error.raw_os_error().is_some() || crate::services::io_error_reason(error).is_some() {
        crate::services::io_error_message(error)
    } else {
        tracing::warn!(%error, "native Recycle Bin operation failed");
        crate::i18n::tr("An unexpected system error occurred")
    }
}

pub(crate) async fn listing() -> Result<strata_win_shell::RecycleBinListing, glib::Error> {
    gio::spawn_blocking(|| strata_win_shell::list_recycled(100_000, Duration::from_secs(30)))
        .await
        .map_err(|_| error(crate::i18n::tr("Unable to read Trash")))?
        .map_err(error)
}

pub(crate) async fn has_items() -> Result<bool, glib::Error> {
    gio::spawn_blocking(|| strata_win_shell::list_recycled(1, Duration::from_secs(5)))
        .await
        .map_err(|_| error(crate::i18n::tr("Unable to read Trash")))?
        .map_err(error)
        .and_then(|listing| {
            if listing.entries.is_empty() && listing.truncated {
                Err(error(crate::i18n::tr("Unable to read Trash")))
            } else {
                Ok(!listing.entries.is_empty())
            }
        })
}

pub(crate) async fn summary() -> Result<super::directory_summary::DirectorySummary, glib::Error> {
    let listing = listing().await?;
    gio::spawn_blocking(move || {
        let mut summary = super::directory_summary::DirectorySummary::default();
        summary.issues.timed_out = listing.truncated;
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut pending = listing
            .entries
            .into_iter()
            .map(|entry| (entry.item.recycled, 0usize))
            .collect::<Vec<_>>();
        while let Some((path, depth)) = pending.pop() {
            if Instant::now() >= deadline || summary.item_count >= 100_000 {
                summary.issues.timed_out = true;
                break;
            }
            let Ok(metadata) = path.symlink_metadata() else {
                summary.issues.unreadable = true;
                continue;
            };
            summary.item_count += 1;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                summary.visible_folder_count += 1;
                if depth >= 64 {
                    summary.issues.depth_limited = true;
                    continue;
                }
                match std::fs::read_dir(path) {
                    Ok(children) => {
                        for child in children {
                            if pending.len() >= 100_000 {
                                summary.issues.depth_limited = true;
                                break;
                            }
                            match child {
                                Ok(child) => pending.push((child.path(), depth + 1)),
                                Err(_) => summary.issues.unreadable = true,
                            }
                        }
                    }
                    Err(_) => summary.issues.unreadable = true,
                }
            } else {
                summary.visible_file_count += 1;
                if metadata.is_file() {
                    summary.total_size = summary.total_size.saturating_add(metadata.len());
                }
            }
        }
        summary
    })
    .await
    .map_err(|_| error(crate::i18n::tr("Unable to read Trash")))
}

pub(crate) fn watch(notify: Rc<dyn Fn(crate::services::DirectoryChange)>) -> LoadHandle {
    let task = glib::MainContext::default().spawn_local(async move {
        let mut previous = None;
        loop {
            if let Ok(listing) = listing().await {
                let mut identities = listing
                    .entries
                    .into_iter()
                    .map(|entry| entry.item.recycled)
                    .collect::<Vec<_>>();
                identities.sort();
                if previous
                    .as_ref()
                    .is_some_and(|previous| previous != &identities)
                {
                    notify(crate::services::DirectoryChange::Rescan);
                }
                previous = Some(identities);
            }
            glib::timeout_future(Duration::from_secs(2)).await;
        }
    });
    LoadHandle::new(move || task.abort())
}

fn entry(item: strata_win_shell::RecycledItem) -> FileEntry {
    let metadata = std::fs::symlink_metadata(&item.recycled).ok();
    let native_name = item.original.file_name().unwrap_or_default().to_os_string();
    FileEntry {
        location: location(&item.recycled),
        display_name: native_name.to_string_lossy().into_owned(),
        native_name,
        thumbnail_path: Some(item.recycled),
        kind: if metadata.as_ref().is_some_and(|metadata| metadata.is_dir()) {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        size: metadata
            .as_ref()
            .map(|metadata| MetadataValue::Known(metadata.len()))
            .unwrap_or(MetadataValue::Unavailable),
        modified_unix_seconds: metadata
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|time| MetadataValue::Known(time.as_secs() as i64))
            .unwrap_or(MetadataValue::Unavailable),
        recent_unix_seconds: MetadataValue::Unknown,
        recent_uri: None,
        mode: MetadataValue::Unavailable,
        image_dimensions: MetadataValue::Unavailable,
        child_count: MetadataValue::Unavailable,
        duration_seconds: MetadataValue::Unavailable,
        is_hidden: false,
    }
}

pub(crate) fn enumerate(request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
    let id = request.id;
    let task = glib::MainContext::default().spawn_local(async move {
        let result = gio::spawn_blocking(move || {
            let listing =
                strata_win_shell::list_recycled(request.max_entries, request.time_budget)?;
            Ok::<_, std::io::Error>((
                listing
                    .entries
                    .into_iter()
                    .map(|item| entry(item.item))
                    .collect::<Vec<_>>(),
                listing.truncated,
                request.batch_size.max(1),
            ))
        })
        .await;
        match result {
            Ok(Ok((entries, truncated, batch_size))) => {
                for batch in entries.chunks(batch_size) {
                    emit(DirectoryEvent::Batch {
                        request_id: id,
                        entries: batch.to_vec(),
                    });
                }
                emit(DirectoryEvent::Finished {
                    request_id: id,
                    truncated,
                    can_trash: Some(false),
                    can_delete: Some(true),
                });
            }
            Ok(Err(error)) => emit(DirectoryEvent::Failed {
                request_id: id,
                message: operation_error_message(&error),
            }),
            Err(_) => emit(DirectoryEvent::Failed {
                request_id: id,
                message: crate::i18n::tr("Unable to read Trash"),
            }),
        }
    });
    LoadHandle::new(move || task.abort())
}

#[cfg(test)]
mod tests;
