// SPDX-License-Identifier: MIT

//! Windows GTK operation-provider bridge. Native file mutations are submitted
//! to the bounded off-main-thread worker service; GTK only polls completions.
//! No shell, volume query, ACL preflight or UAC prompt runs on the UI thread.
//!
//! Operations stay off the GTK thread and use native no-clobber, Recycle Bin,
//! archive, merge, move, and overwrite paths with explicit failure reporting.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};
use strata_native_io::windows_operations::{
    LocalOperation, LocalOperationService, OperationCompletion, SubmitError,
};

use crate::{
    model::Location,
    services::{
        CompressRequest, CreateDirectoryRequest, CreateFileRequest, DeleteRequest, ExtractRequest,
        LoadHandle, OperationEvent, OperationProvider, OperationRequestId, PasteRequest,
        RenameRequest, RestoreRequest, TransferConflict, UndoCopyRequest, UndoMergeRequest,
        UndoMoveRequest, UndoRenameRequest, validate_basename,
    },
};

#[path = "local_operations/archive/listing.rs"]
mod listing;
#[cfg(test)]
#[path = "local_operations_windows/preview_release_tests.rs"]
mod preview_release_tests;

#[cfg(test)]
pub(crate) use listing::ArchiveListing;
pub(crate) use listing::{
    ARCHIVE_PREVIEW_FAILED_MESSAGE, ARCHIVE_TOO_LARGE_MESSAGE, ARCHIVE_UNSUPPORTED_MESSAGE,
    ArchiveListingStatus, INVALID_ARCHIVE, MAX_ARCHIVE_PASSWORD_BYTES, archive_payload_valid,
    decode_archive_listing, encode_archive_result,
};

#[path = "local_operations_windows/archive.rs"]
mod archive;

pub(crate) const PASSWORD_REQUIRED: &str = archive::PASSWORD_REQUIRED;
pub(crate) const MAYBE_BAD_PASSWORD: &str = archive::MAYBE_BAD_PASSWORD;

/// Archive previews must never parse untrusted files without a Windows sandbox.
pub(crate) fn list_archive_entries_direct(
    path: &Path,
    format: crate::services::ArchiveFormat,
    password: Option<&str>,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<listing::ArchiveListing, String> {
    listing::list_archive_entries_direct(path, format, password, cancelled)
}

/// Flushes filesystem buffers on Windows removable volumes before ejection.
pub(crate) fn flush_filesystem(root: &Path) -> io::Result<()> {
    strata_win_shell::flush_volume(root)
}

enum Success {
    Directory,
    Copy {
        source: Location,
        destination: Location,
    },
}
#[derive(Default, Debug)]
struct WindowsMergePlan {
    created: Vec<PathBuf>,
    overwritten: Vec<PathBuf>,
    recycled: Vec<strata_win_shell::RecycledItem>,
}

fn classify_merge_windows(
    source: &Path,
    destination: &Path,
    plan: &mut WindowsMergePlan,
) -> Result<(), String> {
    if !source.is_dir() {
        return Err("Only folders can be merged".to_string());
    }
    if destination.exists() && !destination.is_dir() {
        return Err("A file and a folder cannot safely replace one another".to_string());
    }
    let entries = std::fs::read_dir(source).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let child_source = entry.path();
        let name = entry.file_name();
        let child_dest = destination.join(&name);
        let source_is_dir = entry.file_type().map_err(|e| e.to_string())?.is_dir();
        if !child_dest.exists() {
            plan.created.push(child_dest);
        } else if child_dest.is_dir() && source_is_dir {
            classify_merge_windows(&child_source, &child_dest, plan)?;
        } else if child_dest.is_dir() != source_is_dir {
            return Err("A file and a folder cannot safely replace one another".to_string());
        } else {
            plan.overwritten.push(child_dest);
        }
    }
    Ok(())
}
struct Pending {
    request_id: OperationRequestId,
    success: Success,
    emit: Rc<dyn Fn(OperationEvent)>,
    // Keep a tombstone until the worker result has been received. Removing
    // entries on cancel stops the poller and can fill the bounded result queue.
    cancelled: bool,
}
#[derive(Default)]
struct ProviderState {
    service: RefCell<Option<LocalOperationService>>,
    pending: RefCell<HashMap<u64, Pending>>,
    recycled_overwrites: RefCell<HashMap<PathBuf, strata_win_shell::RecycledItem>>,
    polling: Cell<bool>,
}
// A huge batch of tiny files can complete while GTK is drawing. Never drain
// an unbounded number of result callbacks in a single main-loop iteration:
// favor tactile input latency over one long blocking burst.
const MAX_COMPLETIONS_PER_TICK: usize = 64;
const MAX_COMPLETION_TICK_TIME: Duration = Duration::from_millis(3);

impl ProviderState {
    fn remember_recycled(&self, items: &[strata_win_shell::RecycledItem]) {
        let mut remembered = self.recycled_overwrites.borrow_mut();
        for item in items {
            remembered.insert(item.original.clone(), item.clone());
        }
    }

    fn ensure_polling(self: &Rc<Self>) {
        if self.polling.replace(true) {
            return;
        }
        let weak: Weak<Self> = Rc::downgrade(self);
        // A single source handles all outstanding operations; no source and
        // no worker are created during first-window construction.
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            state.poll();
            if state.pending.borrow().is_empty() {
                state.polling.set(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn poll(&self) {
        let start = Instant::now();
        for _ in 0..MAX_COMPLETIONS_PER_TICK {
            if start.elapsed() >= MAX_COMPLETION_TICK_TIME {
                break;
            }
            let next = self
                .service
                .borrow()
                .as_ref()
                .and_then(|service| service.try_recv().ok());
            let Some(OperationCompletion {
                id,
                result,
                created,
            }) = next
            else {
                break;
            };
            // Drop all RefCell borrows BEFORE invoking user callbacks; a
            // completion can synchronously queue another file operation.
            let pending = self.pending.borrow_mut().remove(&id);
            if let Some(Pending {
                request_id,
                success,
                emit,
                cancelled,
            }) = pending
            {
                if cancelled {
                    // The native worker still sends one completion per request.
                    // Drain it without publishing an event to a dropped handle.
                    continue;
                }
                let event = match result {
                    Ok(()) => match success {
                        Success::Directory => OperationEvent::Created { request_id },
                        Success::Copy {
                            source,
                            destination,
                        } => {
                            // Browser operation tracking records the created
                            // destination from progress, but Pasted holds
                            // SOURCE locations (for selection and move undo).
                            // Keep Both chooses its name on the worker.
                            let destination = created.map_or(destination, Location::local);
                            emit(OperationEvent::TransferProgress {
                                request_id,
                                completed_items: 1,
                                completed_files: 1,
                                total_files: Some(1),
                                current_file: None,
                                transferred_bytes: 0,
                                total_bytes: None,
                                created_location: Some(destination),
                            });
                            OperationEvent::Pasted {
                                request_id,
                                locations: vec![source],
                            }
                        }
                    },
                    Err(error) => OperationEvent::Failed {
                        request_id,
                        message: crate::services::io_error_message(&error),
                        password_failure: None,
                    },
                };
                emit(event);
            }
        }
    }

    fn submit(
        self: &Rc<Self>,
        operation: LocalOperation,
        request_id: OperationRequestId,
        success: Success,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        // Lazily initialize only when a user requests a supported operation.
        // All actual filesystem work happens on the native service workers.
        if self.service.borrow().is_none() {
            let service = match LocalOperationService::new() {
                Ok(service) => service,
                Err(error) => return rejected(request_id, emit, error.to_string()),
            };
            *self.service.borrow_mut() = Some(service);
        }
        let id = match self
            .service
            .borrow()
            .as_ref()
            .expect("initialized")
            .submit(operation)
        {
            Ok(id) => id,
            Err(SubmitError::Busy) => {
                return rejected(request_id, emit, "Windows file worker queue is busy");
            }
            Err(SubmitError::Closed) => {
                return rejected(request_id, emit, "Windows file worker is unavailable");
            }
        };
        self.pending.borrow_mut().insert(
            id,
            Pending {
                request_id,
                success,
                emit,
                cancelled: false,
            },
        );
        self.ensure_polling();
        let weak = Rc::downgrade(self);
        LoadHandle::new(move || {
            if let Some(state) = weak.upgrade() {
                // Only an accepted native cancellation suppresses publication.
                // Leave the tombstone pending until poll() drains its result,
                // otherwise the bounded completion channel can block workers.
                let accepted = state
                    .service
                    .borrow()
                    .as_ref()
                    .is_some_and(|service| service.cancel(id));
                if accepted && let Some(pending) = state.pending.borrow_mut().get_mut(&id) {
                    pending.cancelled = true;
                }
            }
        })
    }
}

/// A declined destructive operation must never masquerade as success.
fn rejected(
    request_id: OperationRequestId,
    emit: Rc<dyn Fn(OperationEvent)>,
    message: impl Into<String>,
) -> LoadHandle {
    emit(OperationEvent::Failed {
        request_id,
        message: message.into(),
        password_failure: None,
    });
    LoadHandle::new(|| {})
}

#[derive(Default)]
pub struct LocalOperationProvider {
    state: Rc<ProviderState>,
}

impl OperationProvider for LocalOperationProvider {
    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        if let Err(message) = validate_basename(&request.name) {
            return rejected(request.id, emit, crate::i18n::tr(message));
        }
        if request.unique_name {
            // GIO make_directory_async publishes the exact successful sibling
            // after atomic no-clobber creation. Do not preflight exists().
            return async_create_directory(request, emit);
        }
        let Some(parent) = request.parent.native_path() else {
            return rejected(
                request.id,
                emit,
                "Windows native folder creation requires a local path",
            );
        };
        let destination = parent.join(&request.name);
        if !strata_native_io::windows_paths::valid_child_name(OsStr::new(&request.name)) {
            return rejected(request.id, emit, "Invalid Windows folder name");
        }
        self.state.submit(
            LocalOperation::CreateDirectory(destination),
            request.id,
            Success::Directory,
            emit,
        )
    }

    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        // GIO's async create uses no-overwrite semantics and can return the
        // actual unique filename; it does not call our synchronous workers.
        async_create_file(request, emit)
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        if request.move_sources {
            return async_move_items(self.state.clone(), request, emit);
        }
        async_copy_items(self.state.clone(), request, emit)
    }

    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        if let Err(message) = validate_basename(&request.new_name) {
            return rejected(request.id, emit, crate::i18n::tr(message));
        }
        if !strata_native_io::windows_paths::valid_child_name(OsStr::new(&request.new_name)) {
            return rejected(request.id, emit, "Invalid Windows filename");
        }
        let Some(source) = request.entry.location.native_path() else {
            return rejected(request.id, emit, "Windows rename requires a local path");
        };
        let Some(parent) = source.parent() else {
            return rejected(request.id, emit, "Cannot rename a volume root");
        };
        // Rename is a same-directory move: never use std::fs::rename here,
        // whose Windows replacement semantics are not a no-clobber contract.
        let target = parent.join(&request.new_name);
        async_rename_no_replace(source.to_path_buf(), target, request.id, emit)
    }
    fn undo_move(&self, request: UndoMoveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let mut operations = Vec::with_capacity(request.items.len());
        for item in &request.items {
            let (Some(source), Some(target)) = (
                item.record.current.native_path(),
                item.record.original.native_path(),
            ) else {
                return rejected(request.id, emit, "Windows undo move requires local paths");
            };
            operations.push((
                item.record.current.clone(),
                source.to_path_buf(),
                target.to_path_buf(),
            ));
        }
        async_move_paths(
            Some(self.state.clone()),
            request.id,
            operations,
            request.cleanup_locations,
            emit,
        )
    }
    fn undo_rename(
        &self,
        request: UndoRenameRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let (Some(source), Some(target)) = (
            request.current.native_path(),
            request.original.native_path(),
        ) else {
            return rejected(request.id, emit, "Windows undo rename requires local paths");
        };
        if source.parent().is_none() || source.parent() != target.parent() {
            return rejected(
                request.id,
                emit,
                "Undo rename must stay in the original folder",
            );
        }
        if !target
            .file_name()
            .is_some_and(strata_native_io::windows_paths::valid_child_name)
        {
            return rejected(request.id, emit, "Invalid Windows filename");
        }
        async_rename_no_replace(source.to_path_buf(), target.to_path_buf(), request.id, emit)
    }
    fn undo_copy(&self, request: UndoCopyRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        recycle_locations(request.id, request.locations, emit)
    }
    fn undo_merge(
        &self,
        request: UndoMergeRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        run_undo_merge(self.state.clone(), request, emit)
    }
    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        if request.permanent {
            let locations = request
                .entries
                .into_iter()
                .map(|entry| entry.location)
                .collect();
            return permanently_delete_locations(request.id, locations, emit);
        }
        let locations = request.entries.into_iter().map(|e| e.location).collect();
        recycle_locations(request.id, locations, emit)
    }
    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let locations = match request.source {
            crate::services::RestoreSource::OriginalLocations(locations) => locations
                .into_iter()
                .map(|location| (location, None))
                .collect(),
            crate::services::RestoreSource::TrashEntries(items) => items
                .into_iter()
                .map(|item| (item.entry.location, Some(item.destination)))
                .collect(),
        };
        restore_selected_locations(request.id, locations, emit)
    }
    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        archive::compress(request, emit)
    }
    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        archive::extract(request, emit)
    }
}

fn permanently_delete_locations(
    id: OperationRequestId,
    locations: Vec<Location>,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    let mut paths = Vec::with_capacity(locations.len());
    for location in &locations {
        let path = location.native_path().map(Path::to_path_buf).or_else(|| {
            super::recycle_bin::is_item(location)
                .then(|| super::recycle_bin::path(location))
                .flatten()
        });
        let Some(path) = path else {
            return rejected(id, emit, "Windows delete requires local paths");
        };
        paths.push((location.clone(), path));
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    glib::MainContext::default().spawn_local(async move {
        let mut roots = locations;
        roots.extend(paths.iter().map(|item| Location::local(&item.1)));
        crate::services::file_mutations::release(&roots).await;

        let outcome = gio::spawn_blocking(move || {
            let mut deleted = Vec::new();
            let mut failed = Vec::new();
            let mut not_attempted = Vec::new();
            for (location, path) in paths {
                if worker_cancelled.load(AtomicOrdering::Acquire) {
                    not_attempted.push(location);
                    continue;
                }
                let outcome = if super::recycle_bin::is_item(&location) {
                    strata_win_shell::delete_recycled(&path)
                } else {
                    strata_win_shell::permanently_delete(&[path])
                };
                match outcome {
                    Ok(()) => deleted.push(location),
                    Err(error) => failed.push((location, error)),
                }
            }
            (deleted, failed, not_attempted)
        })
        .await;
        let Ok((deleted, failed, not_attempted)) = outcome else {
            emit(OperationEvent::Failed {
                request_id: id,
                message: "Recycle Bin worker stopped".to_owned(),
                password_failure: None,
            });
            return;
        };
        if !not_attempted.is_empty() {
            emit(OperationEvent::Cancelled {
                request_id: id,
                result: crate::services::CancelledOperation {
                    completed: deleted,
                    failed: failed.into_iter().map(|(location, _)| location).collect(),
                    not_attempted,
                    affected_locations: std::collections::HashSet::new(),
                },
            });
        } else if let Some((_, error)) = failed.first() {
            let message = super::recycle_bin::operation_error_message(error);
            if deleted.is_empty() {
                emit(OperationEvent::Failed {
                    request_id: id,
                    message,
                    password_failure: None,
                });
            } else {
                emit(OperationEvent::CompletedWithErrors {
                    request_id: id,
                    deleted_locations: deleted,
                    retryable_locations: Vec::new(),
                    has_non_retryable_failures: true,
                    message,
                });
            }
        } else {
            emit(OperationEvent::Deleted {
                request_id: id,
                locations: deleted,
            });
        }
    });
    LoadHandle::new(move || cancelled.store(true, AtomicOrdering::Release))
}

async fn submit_copy_and_await(
    state: &Rc<ProviderState>,
    operation: LocalOperation,
    id: OperationRequestId,
    source: &Location,
    destination: &Path,
    cancel: &gio::Cancellable,
) -> Result<Option<Location>, String> {
    let (sender, receiver) = futures_channel::oneshot::channel();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let destination_loc = Location::local(destination.to_path_buf());
    let callback: Rc<dyn Fn(OperationEvent)> = Rc::new(move |event| {
        let outcome = match event {
            OperationEvent::TransferProgress {
                created_location, ..
            } => Ok(created_location),
            OperationEvent::Pasted { .. } => Ok(None),
            OperationEvent::Failed { message, .. } => Err(message),
            _ => return,
        };
        if let Some(sender) = sender.borrow_mut().take() {
            let _ = sender.send(outcome);
        }
    });

    let handle = Rc::new(RefCell::new(Some(state.submit(
        operation,
        id,
        Success::Copy {
            source: source.clone(),
            destination: destination_loc,
        },
        callback,
    ))));

    let cancel_handle = handle.clone();
    let signal = cancel.connect_cancelled_local(move |_| {
        cancel_handle.borrow_mut().take();
    });
    let outcome = receiver.await;
    if let Some(signal) = signal {
        cancel.disconnect_cancelled(signal);
    }
    handle.borrow_mut().take();
    match outcome {
        Ok(res) => res,
        Err(_) => Err("Operation cancelled or worker channel dropped".to_string()),
    }
}

fn async_copy_items(
    state: Rc<ProviderState>,
    request: PasteRequest,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    use strata_native_io::windows_paths::{PathKind, classify, valid_child_name};

    let Some(destination_dir) = request.destination.native_path() else {
        return rejected(
            request.id,
            emit,
            "Windows copy requires a local destination",
        );
    };
    if !matches!(
        classify(destination_dir),
        PathKind::LocalAbsolute | PathKind::NetworkAbsolute
    ) {
        return rejected(request.id, emit, "Invalid Windows destination");
    }

    let mut operations = Vec::with_capacity(request.items.len());
    for item in &request.items {
        let Some(source) = item.source.native_path() else {
            return rejected(request.id, emit, "Windows copy requires local source paths");
        };
        if !matches!(
            classify(source),
            PathKind::LocalAbsolute | PathKind::NetworkAbsolute
        ) || !source.file_name().is_some_and(valid_child_name)
        {
            return rejected(request.id, emit, "Invalid Windows source filename");
        }
        let name = source.file_name().expect("validated");
        let destination = destination_dir.join(name);
        operations.push((
            item.source.clone(),
            source.to_path_buf(),
            destination,
            item.conflict,
        ));
    }

    let total = operations.len();
    if total == 0 {
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: Vec::new(),
        });
        return LoadHandle::new(|| {});
    }

    let cancel = gio::Cancellable::new();
    let task_cancel = cancel.clone();
    glib::MainContext::default().spawn_local(async move {
        crate::services::file_mutations::release(
            &operations
                .iter()
                .filter(|item| {
                    matches!(
                        item.3,
                        TransferConflict::ReplaceExisting | TransferConflict::Merge
                    )
                })
                .map(|item| Location::local(&item.2))
                .collect::<Vec<_>>(),
        )
        .await;

        let mut completed = Vec::new();
        let not_attempted_items = operations.clone();
        for (index, (source_loc, source_path, destination_path, conflict)) in
            operations.into_iter().enumerate()
        {
            if task_cancel.is_cancelled() {
                emit(OperationEvent::Cancelled {
                    request_id: request.id,
                    result: crate::services::CancelledOperation {
                        completed,
                        not_attempted: not_attempted_items[index..]
                            .iter()
                            .map(|item| item.0.clone())
                            .collect(),
                        affected_locations: std::collections::HashSet::new(),
                        ..Default::default()
                    },
                });
                return;
            }

            let pre_res = gio::spawn_blocking({
                let src = source_path.clone();
                let dst = destination_path.clone();
                move || -> Result<(LocalOperation, Option<WindowsMergePlan>), String> {
                    match conflict {
                        TransferConflict::FailIfExists => Ok((
                            LocalOperation::CopyPath {
                                source: src,
                                destination: dst,
                            },
                            None,
                        )),
                        TransferConflict::KeepBoth => Ok((
                            LocalOperation::CopyPathKeepBoth {
                                source: src,
                                destination: dst,
                            },
                            None,
                        )),
                        TransferConflict::ReplaceExisting => {
                            let mut plan = None;
                            if dst.exists() {
                                let recycled = strata_win_shell::recycle(&[dst.clone()])
                                    .map_err(|e| e.to_string())?;
                                plan = Some(WindowsMergePlan {
                                    created: Vec::new(),
                                    overwritten: vec![dst.clone()],
                                    recycled,
                                });
                            }
                            Ok((
                                LocalOperation::CopyPath {
                                    source: src,
                                    destination: dst,
                                },
                                plan,
                            ))
                        }
                        TransferConflict::Merge => {
                            if dst.exists() {
                                let mut plan = WindowsMergePlan::default();
                                classify_merge_windows(&src, &dst, &mut plan)?;
                                if !plan.overwritten.is_empty() {
                                    plan.recycled = strata_win_shell::recycle(&plan.overwritten)
                                        .map_err(|e| e.to_string())?;
                                }
                                Ok((
                                    LocalOperation::MergePath {
                                        source: src,
                                        destination: dst,
                                    },
                                    Some(plan),
                                ))
                            } else {
                                let plan = WindowsMergePlan {
                                    created: vec![dst.clone()],
                                    overwritten: Vec::new(),
                                    recycled: Vec::new(),
                                };
                                Ok((
                                    LocalOperation::CopyPath {
                                        source: src,
                                        destination: dst,
                                    },
                                    Some(plan),
                                ))
                            }
                        }
                    }
                }
            })
            .await;

            let (operation, merge_plan) = match pre_res {
                Ok(Ok(pair)) => pair,
                Ok(Err(err)) => {
                    emit(OperationEvent::TransferFailed {
                        request_id: request.id,
                        completed_locations: completed,
                        message: err,
                    });
                    return;
                }
                Err(_) => {
                    emit(OperationEvent::TransferFailed {
                        request_id: request.id,
                        completed_locations: completed,
                        message: "Pre-copy worker error".to_string(),
                    });
                    return;
                }
            };

            if let Some(plan) = merge_plan {
                state.remember_recycled(&plan.recycled);
                emit(OperationEvent::Merged {
                    request_id: request.id,
                    source: source_loc.clone(),
                    created: plan.created.into_iter().map(Location::local).collect(),
                    overwritten: plan.overwritten.into_iter().map(Location::local).collect(),
                });
            }

            let submit_res = submit_copy_and_await(
                &state,
                operation,
                request.id,
                &source_loc,
                &destination_path,
                &task_cancel,
            )
            .await;
            match submit_res {
                Ok(created_dest) => {
                    let dest_location =
                        created_dest.unwrap_or_else(|| Location::local(destination_path));
                    completed.push(source_loc);
                    emit(OperationEvent::TransferProgress {
                        request_id: request.id,
                        completed_items: completed.len(),
                        completed_files: completed.len(),
                        total_files: Some(total),
                        current_file: None,
                        transferred_bytes: 0,
                        total_bytes: None,
                        created_location: Some(dest_location),
                    });
                }
                Err(error) => {
                    if task_cancel.is_cancelled() {
                        emit(OperationEvent::Cancelled {
                            request_id: request.id,
                            result: crate::services::CancelledOperation {
                                completed,
                                not_attempted: not_attempted_items[index..]
                                    .iter()
                                    .map(|item| item.0.clone())
                                    .collect(),
                                affected_locations: std::collections::HashSet::new(),
                                ..Default::default()
                            },
                        });
                    } else {
                        emit(OperationEvent::TransferFailed {
                            request_id: request.id,
                            completed_locations: completed,
                            message: error,
                        });
                    }
                    return;
                }
            }
        }
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: completed,
        });
    });

    LoadHandle::new(move || cancel.cancel())
}

/// All transfer paths are validated before the first mutation. A batch
/// reports exactly which source items moved if a later item fails.
fn async_move_items(
    state: Rc<ProviderState>,
    request: PasteRequest,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    use strata_native_io::windows_paths::{
        PathKind, classify, numbered_candidate, valid_child_name,
    };

    let Some(destination_dir) = request.destination.native_path() else {
        return rejected(
            request.id,
            emit,
            "Windows move requires a local destination",
        );
    };
    if !matches!(
        classify(destination_dir),
        PathKind::LocalAbsolute | PathKind::NetworkAbsolute
    ) {
        return rejected(request.id, emit, "Invalid Windows destination");
    }

    let mut items = Vec::with_capacity(request.items.len());
    for item in &request.items {
        let Some(source) = item.source.native_path() else {
            return rejected(request.id, emit, "Windows move requires local source paths");
        };
        let Some(name) = source.file_name() else {
            return rejected(request.id, emit, "Cannot move a volume root");
        };
        if !valid_child_name(name) {
            return rejected(request.id, emit, "Invalid Windows filename");
        }
        let target = destination_dir.join(name);
        items.push((
            item.source.clone(),
            source.to_path_buf(),
            target,
            item.conflict,
        ));
    }

    let cancel = gio::Cancellable::new();
    let task_cancel = cancel.clone();
    let total = items.len();
    if total == 0 {
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: Vec::new(),
        });
        return LoadHandle::new(|| {});
    }

    glib::MainContext::default().spawn_local(async move {
        let mut roots = items.iter().map(|item| item.0.clone()).collect::<Vec<_>>();
        roots.extend(
            items
                .iter()
                .filter(|item| {
                    matches!(
                        item.3,
                        TransferConflict::ReplaceExisting | TransferConflict::Merge
                    )
                })
                .map(|item| Location::local(&item.2)),
        );
        crate::services::file_mutations::release(&roots).await;

        let mut completed = Vec::new();
        let not_attempted_items = items.clone();
        for (index, (location, source, mut target, conflict)) in items.into_iter().enumerate() {
            if task_cancel.is_cancelled() {
                emit(OperationEvent::Cancelled {
                    request_id: request.id,
                    result: crate::services::CancelledOperation {
                        completed,
                        not_attempted: not_attempted_items[index..]
                            .iter()
                            .map(|item| item.0.clone())
                            .collect(),
                        affected_locations: std::collections::HashSet::new(),
                        ..Default::default()
                    },
                });
                return;
            }

            let pre_res = gio::spawn_blocking({
                let src = source.clone();
                let dst = target.clone();
                move || -> Result<(PathBuf, bool, Option<WindowsMergePlan>), String> {
                    match conflict {
                        TransferConflict::FailIfExists => Ok((dst, false, None)),
                        TransferConflict::KeepBoth => {
                            if dst.exists() {
                                let is_file = std::fs::symlink_metadata(&src)
                                    .map(|m| m.is_file())
                                    .unwrap_or(true);
                                let mut candidate = dst.clone();
                                for idx in 1..=10_000 {
                                    candidate = numbered_candidate(&dst, idx, is_file);
                                    if !candidate.exists() {
                                        break;
                                    }
                                }
                                Ok((candidate, false, None))
                            } else {
                                Ok((dst, false, None))
                            }
                        }
                        TransferConflict::ReplaceExisting => {
                            let mut plan = None;
                            if dst.exists() {
                                let recycled = strata_win_shell::recycle(&[dst.clone()])
                                    .map_err(|e| e.to_string())?;
                                plan = Some(WindowsMergePlan {
                                    created: Vec::new(),
                                    overwritten: vec![dst.clone()],
                                    recycled,
                                });
                            }
                            Ok((dst, false, plan))
                        }
                        TransferConflict::Merge => {
                            if dst.exists() {
                                let mut plan = WindowsMergePlan::default();
                                classify_merge_windows(&src, &dst, &mut plan)?;
                                if !plan.overwritten.is_empty() {
                                    plan.recycled = strata_win_shell::recycle(&plan.overwritten)
                                        .map_err(|e| e.to_string())?;
                                }
                                Ok((dst, true, Some(plan)))
                            } else {
                                let plan = WindowsMergePlan {
                                    created: vec![dst.clone()],
                                    overwritten: Vec::new(),
                                    recycled: Vec::new(),
                                };
                                Ok((dst, false, Some(plan)))
                            }
                        }
                    }
                }
            })
            .await;

            let (resolved_target, is_merge_dir, merge_plan) = match pre_res {
                Ok(Ok(triplet)) => triplet,
                Ok(Err(err)) => {
                    emit(OperationEvent::TransferFailed {
                        request_id: request.id,
                        completed_locations: completed,
                        message: err,
                    });
                    return;
                }
                Err(_) => {
                    emit(OperationEvent::TransferFailed {
                        request_id: request.id,
                        completed_locations: completed,
                        message: "Pre-move worker error".to_string(),
                    });
                    return;
                }
            };
            target = resolved_target;

            if let Some(plan) = merge_plan {
                state.remember_recycled(&plan.recycled);
                emit(OperationEvent::Merged {
                    request_id: request.id,
                    source: location.clone(),
                    created: plan.created.into_iter().map(Location::local).collect(),
                    overwritten: plan.overwritten.into_iter().map(Location::local).collect(),
                });
            }

            let move_outcome = if is_merge_dir {
                let merge_res = submit_copy_and_await(
                    &state,
                    LocalOperation::MergePath {
                        source: source.clone(),
                        destination: target.clone(),
                    },
                    request.id,
                    &location,
                    &target,
                    &task_cancel,
                )
                .await;
                match merge_res {
                    Ok(_) => {
                        let del_res = gio::spawn_blocking({
                            let src = source.clone();
                            move || std::fs::remove_dir_all(&src)
                        })
                        .await;
                        match del_res {
                            Ok(Ok(())) => Ok(()),
                            Ok(Err(e)) => {
                                Err(glib::Error::new(gio::IOErrorEnum::Failed, &e.to_string()))
                            }
                            Err(_) => Err(glib::Error::new(
                                gio::IOErrorEnum::Failed,
                                "Delete source folder failed",
                            )),
                        }
                    }
                    Err(e) => Err(glib::Error::new(gio::IOErrorEnum::Failed, &e)),
                }
            } else if source == target {
                Ok(())
            } else {
                match move_no_replace(&source, &target, task_cancel.clone()).await {
                    Err(error) if error.matches(gio::IOErrorEnum::NotSupported) => {
                        move_verified_native(&state, &source, &target, request.id, &task_cancel)
                            .await
                    }
                    outcome => outcome,
                }
            };

            match move_outcome {
                Ok(()) => {
                    let target_location = Location::local(target.clone());
                    emit(OperationEvent::ItemMoved {
                        request_id: request.id,
                        from: location.clone(),
                        to: target_location,
                        merged: is_merge_dir,
                    });
                    completed.push(location.clone());
                    emit(OperationEvent::TransferProgress {
                        request_id: request.id,
                        completed_items: completed.len(),
                        completed_files: completed.len(),
                        total_files: Some(total),
                        current_file: None,
                        transferred_bytes: 0,
                        total_bytes: None,
                        created_location: None,
                    });
                }
                Err(error) if error.matches(gio::IOErrorEnum::Cancelled) => {
                    emit(OperationEvent::Cancelled {
                        request_id: request.id,
                        result: crate::services::CancelledOperation {
                            completed,
                            not_attempted: not_attempted_items[index..]
                                .iter()
                                .map(|op| op.0.clone())
                                .collect(),
                            affected_locations: std::collections::HashSet::new(),
                            ..Default::default()
                        },
                    });
                    return;
                }
                Err(error) => {
                    emit(OperationEvent::TransferFailed {
                        request_id: request.id,
                        completed_locations: completed,
                        message: crate::services::gio_error_message(&error),
                    });
                    return;
                }
            }
        }
        emit(OperationEvent::Pasted {
            request_id: request.id,
            locations: completed,
        });
    });

    LoadHandle::new(move || cancel.cancel())
}

/// A single validated executor is shared by move and undo-move, keeping
/// failure/cancellation accounting consistent across both directions.
fn async_move_paths(
    state: Option<Rc<ProviderState>>,
    id: OperationRequestId,
    operations: Vec<(Location, std::path::PathBuf, std::path::PathBuf)>,
    cleanup_locations: Vec<Location>,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    use strata_native_io::windows_paths::{PathKind, classify, valid_child_name};

    // Validate every item before touching any of them. In particular, refuse
    // device namespace and drive-relative paths even if passed by a caller
    // other than the ordinary browser UI.
    for (_, source, target) in &operations {
        for path in [source, target] {
            if !matches!(
                classify(path),
                PathKind::LocalAbsolute | PathKind::NetworkAbsolute
            ) || !path.file_name().is_some_and(valid_child_name)
            {
                return rejected(id, emit, "Invalid Windows move source or destination");
            }
        }
    }

    let cancel = gio::Cancellable::new();
    let task_cancel = cancel.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        crate::services::file_mutations::release(
            &operations
                .iter()
                .map(|item| item.0.clone())
                .collect::<Vec<_>>(),
        )
        .await;

        let mut completed = Vec::new();
        let mut affected = std::collections::HashSet::new();
        for (_, source, target) in &operations {
            for path in [source, target] {
                if let Some(parent) = path.parent() {
                    affected.insert(Location::local(parent.to_path_buf()));
                }
            }
        }
        for (index, (location, source, target)) in operations.iter().enumerate() {
            if task_cancel.is_cancelled() {
                emit(OperationEvent::Cancelled {
                    request_id: id,
                    result: crate::services::CancelledOperation {
                        completed,
                        not_attempted: operations[index..].iter().map(|op| op.0.clone()).collect(),
                        affected_locations: affected,
                        ..Default::default()
                    },
                });
                return;
            }
            let outcome = if source == target {
                Ok(())
            } else {
                match move_no_replace(source, target, task_cancel.clone()).await {
                    // A same-volume rename is impossible (other volume or a
                    // filesystem without rename). Fall back only to the
                    // no-clobber copy, verify, then delete-source worker.
                    Err(error) if error.matches(gio::IOErrorEnum::NotSupported) => {
                        match state.as_ref() {
                            Some(state) => {
                                move_verified_native(state, source, target, id, &task_cancel).await
                            }
                            None => Err(error),
                        }
                    }
                    outcome => outcome,
                }
            };
            match outcome {
                Ok(()) => {
                    if source != target {
                        let target_location = Location::local(target.clone());
                        affected.insert(target_location.clone());
                        emit(OperationEvent::ItemMoved {
                            request_id: id,
                            from: location.clone(),
                            to: target_location,
                            merged: false,
                        });
                    }
                    completed.push(location.clone());
                    emit(OperationEvent::TransferProgress {
                        request_id: id,
                        completed_items: completed.len(),
                        completed_files: completed.len(),
                        total_files: Some(operations.len()),
                        current_file: None,
                        transferred_bytes: 0,
                        total_bytes: None,
                        created_location: None,
                    });
                }
                Err(error) if error.matches(gio::IOErrorEnum::Cancelled) => {
                    emit(OperationEvent::Cancelled {
                        request_id: id,
                        result: crate::services::CancelledOperation {
                            completed,
                            not_attempted: operations[index..]
                                .iter()
                                .map(|op| op.0.clone())
                                .collect(),
                            affected_locations: affected,
                            ..Default::default()
                        },
                    });
                    return;
                }
                Err(error) => {
                    emit(OperationEvent::TransferFailed {
                        request_id: id,
                        completed_locations: completed,
                        message: crate::services::gio_error_message(&error),
                    });
                    return;
                }
            }
        }
        if !cleanup_locations.is_empty() {
            let mut cleanup_paths = Vec::new();
            for loc in cleanup_locations {
                if let Some(path) = loc.native_path()
                    && path.exists()
                {
                    cleanup_paths.push(path.to_path_buf());
                }
            }
            if !cleanup_paths.is_empty() {
                let _ = gio::spawn_blocking(move || {
                    let _ = strata_win_shell::recycle(&cleanup_paths);
                })
                .await;
            }
        }
        emit(OperationEvent::Pasted {
            request_id: id,
            locations: completed,
        });
    });
    LoadHandle::new(move || cancel.cancel())
}

/// Move entries to the Recycle Bin one at a time on a blocking worker, so a
/// partial failure reports exactly which items are in the bin. Nothing here
/// falls back to permanent deletion.
fn recycle_locations(
    id: OperationRequestId,
    locations: Vec<Location>,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    let mut items = Vec::with_capacity(locations.len());
    for location in &locations {
        let Some(path) = location.native_path() else {
            return rejected(id, emit, "Windows delete requires local paths");
        };
        items.push((location.clone(), path.to_path_buf()));
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let affected: std::collections::HashSet<Location> = items
        .iter()
        .filter_map(|(_, path)| {
            path.parent()
                .map(|parent| Location::local(parent.to_path_buf()))
        })
        .collect();
    glib::MainContext::default().spawn_local(async move {
        crate::services::file_mutations::release(&locations).await;

        let outcome = gio::spawn_blocking(move || {
            let mut deleted = Vec::new();
            let mut failed = Vec::new();
            let mut not_attempted = Vec::new();
            for (location, path) in items {
                if worker_cancelled.load(AtomicOrdering::Acquire) {
                    not_attempted.push(location);
                    continue;
                }
                match strata_win_shell::recycle(&[path]) {
                    Ok(_) => deleted.push(location),
                    Err(error) => failed.push((location, error)),
                }
            }
            (deleted, failed, not_attempted)
        })
        .await;
        let Ok((deleted, failed, not_attempted)) = outcome else {
            emit(OperationEvent::Failed {
                request_id: id,
                message: crate::services::io_error_message(&io::Error::other(
                    "Recycle Bin worker stopped",
                )),
                password_failure: None,
            });
            return;
        };
        if !not_attempted.is_empty() {
            emit(OperationEvent::Cancelled {
                request_id: id,
                result: crate::services::CancelledOperation {
                    completed: deleted,
                    failed: failed.into_iter().map(|(location, _)| location).collect(),
                    not_attempted,
                    affected_locations: affected,
                },
            });
        } else if let Some((_, error)) = failed.first() {
            let message = crate::services::io_error_message(error);
            if deleted.is_empty() {
                emit(OperationEvent::Failed {
                    request_id: id,
                    message,
                    password_failure: None,
                });
            } else {
                emit(OperationEvent::CompletedWithErrors {
                    request_id: id,
                    deleted_locations: deleted,
                    retryable_locations: Vec::new(),
                    has_non_retryable_failures: true,
                    message,
                });
            }
        } else {
            emit(OperationEvent::Deleted {
                request_id: id,
                locations: deleted,
            });
        }
    });
    LoadHandle::new(move || cancelled.store(true, AtomicOrdering::Release))
}

#[cfg(test)]
fn restore_locations(
    id: OperationRequestId,
    locations: Vec<Location>,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    restore_selected_locations(
        id,
        locations
            .into_iter()
            .map(|location| (location, None))
            .collect(),
        emit,
    )
}

fn restore_selected_locations(
    id: OperationRequestId,
    locations: Vec<(Location, Option<PathBuf>)>,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    let mut paths = Vec::with_capacity(locations.len());
    for (location, destination) in &locations {
        let path = location.native_path().map(Path::to_path_buf).or_else(|| {
            super::recycle_bin::is_item(location)
                .then(|| super::recycle_bin::path(location))
                .flatten()
        });
        let Some(path) = path else {
            return rejected(id, emit, "Windows restore requires local paths");
        };
        paths.push((location.clone(), path, destination.clone()));
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let total = paths.len();
    glib::MainContext::default().spawn_local(async move {
        let roots = paths
            .iter()
            .flat_map(|item| [item.0.clone(), Location::local(&item.1)])
            .collect::<Vec<_>>();
        crate::services::file_mutations::release(&roots).await;

        let outcome = gio::spawn_blocking(move || {
            let mut restored = Vec::new();
            let mut failed = Vec::new();
            let mut not_attempted = Vec::new();
            for (location, path, expected_destination) in paths {
                if worker_cancelled.load(AtomicOrdering::Acquire) {
                    not_attempted.push(location);
                    continue;
                }
                let outcome = if super::recycle_bin::is_item(&location) {
                    strata_win_shell::original_path(&path).and_then(|original| {
                        if expected_destination
                            .as_ref()
                            .is_some_and(|expected| expected != &original)
                        {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "restore destination changed",
                            ));
                        }
                        strata_win_shell::restore(&strata_win_shell::RecycledItem {
                            original: original.clone(),
                            recycled: path,
                        })?;
                        Ok(original)
                    })
                } else {
                    strata_win_shell::restore_by_original_path(&path)
                };
                match outcome {
                    Ok(dest) => restored.push((location, Location::local(dest))),
                    Err(error) => failed.push((location, error)),
                }
            }
            (restored, failed, not_attempted)
        })
        .await;

        let Ok((restored, failed, not_attempted)) = outcome else {
            emit(OperationEvent::Failed {
                request_id: id,
                message: crate::services::io_error_message(&io::Error::other(
                    "Recycle Bin restore worker stopped",
                )),
                password_failure: None,
            });
            return;
        };

        let restored_srcs: Vec<Location> = restored.iter().map(|(src, _)| src.clone()).collect();
        let restored_dests: Vec<Location> = restored.iter().map(|(_, dst)| dst.clone()).collect();

        if !not_attempted.is_empty() {
            emit(OperationEvent::Cancelled {
                request_id: id,
                result: crate::services::CancelledOperation {
                    completed: restored_dests,
                    failed: failed.into_iter().map(|(location, _)| location).collect(),
                    not_attempted,
                    affected_locations: std::collections::HashSet::new(),
                },
            });
        } else if let Some((_, error)) = failed.first() {
            let message = super::recycle_bin::operation_error_message(error);
            if restored.is_empty() {
                emit(OperationEvent::Failed {
                    request_id: id,
                    message,
                    password_failure: None,
                });
            } else {
                emit(OperationEvent::RestoreCompletedWithErrors {
                    request_id: id,
                    restored_locations: restored_srcs,
                    restored: restored_dests,
                    message,
                });
            }
        } else {
            for (i, (_, dst)) in restored.iter().enumerate() {
                emit(OperationEvent::RestoreProgress {
                    request_id: id,
                    completed: i + 1,
                    total,
                    restored_location: Some(dst.clone()),
                });
            }
            emit(OperationEvent::Restored {
                request_id: id,
                locations: restored_srcs,
                restored: restored_dests,
            });
        }
    });
    LoadHandle::new(move || cancelled.store(true, AtomicOrdering::Release))
}

fn run_undo_merge(
    state: Rc<ProviderState>,
    request: UndoMergeRequest,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let task_cancelled = cancelled.clone();
    let total = request.overwritten.len() + request.created.len();
    let id = request.id;
    let remembered = {
        let mut remembered = state.recycled_overwrites.borrow_mut();
        request
            .overwritten
            .iter()
            .filter_map(|location| {
                let path = location.native_path()?.to_path_buf();
                remembered.remove(&path).map(|item| (path, item))
            })
            .collect::<HashMap<_, _>>()
    };
    glib::MainContext::default().spawn_local(async move {
        let roots = request
            .created
            .iter()
            .chain(&request.overwritten)
            .cloned()
            .collect::<Vec<_>>();
        crate::services::file_mutations::release(&roots).await;

        let outcome = gio::spawn_blocking(move || {
            let mut completed_count = 0usize;
            let mut failed = Vec::new();
            let mut restored = Vec::new();

            // 1. Overwritten items: delete the incoming copy, then restore original from Recycle Bin
            for location in &request.overwritten {
                if worker_cancelled.load(AtomicOrdering::Acquire) {
                    break;
                }
                let Some(path) = location.native_path() else {
                    failed.push((
                        location.clone(),
                        io::Error::new(io::ErrorKind::InvalidInput, "non-native path"),
                    ));
                    continue;
                };
                if path.symlink_metadata().is_ok() {
                    if let Err(e) = strata_win_shell::recycle(&[path.to_path_buf()]) {
                        failed.push((location.clone(), e));
                        continue;
                    }
                }
                let restore_result = match remembered.get(path) {
                    Some(item) => strata_win_shell::restore(item).map(|()| item.original.clone()),
                    None => strata_win_shell::restore_by_original_path(path),
                };
                match restore_result {
                    Ok(dest) => {
                        completed_count += 1;
                        restored.push(Location::local(dest));
                    }
                    Err(e) => {
                        failed.push((location.clone(), e));
                    }
                }
            }

            // 2. Created items: send to Recycle Bin
            for location in &request.created {
                if worker_cancelled.load(AtomicOrdering::Acquire) {
                    break;
                }
                let Some(path) = location.native_path() else {
                    failed.push((
                        location.clone(),
                        io::Error::new(io::ErrorKind::InvalidInput, "non-native path"),
                    ));
                    continue;
                };
                if path.exists() {
                    match strata_win_shell::recycle(&[path.to_path_buf()]) {
                        Ok(_) => {
                            completed_count += 1;
                        }
                        Err(e) => {
                            failed.push((location.clone(), e));
                        }
                    }
                } else {
                    completed_count += 1;
                }
            }

            (completed_count, restored, failed)
        })
        .await;

        let Ok((completed_count, restored, failed)) = outcome else {
            emit(OperationEvent::Failed {
                request_id: id,
                message: "Undo merge worker stopped".to_string(),
                password_failure: None,
            });
            return;
        };

        if task_cancelled.load(AtomicOrdering::Acquire) {
            emit(OperationEvent::Cancelled {
                request_id: id,
                result: crate::services::CancelledOperation {
                    completed: restored,
                    failed: failed.into_iter().map(|(l, _)| l).collect(),
                    not_attempted: Vec::new(),
                    affected_locations: std::collections::HashSet::new(),
                },
            });
            return;
        }

        if let Some((_, error)) = failed.first() {
            let message = crate::services::io_error_message(error);
            if completed_count == 0 {
                emit(OperationEvent::Failed {
                    request_id: id,
                    message,
                    password_failure: None,
                });
            } else {
                emit(OperationEvent::CompletedWithErrors {
                    request_id: id,
                    deleted_locations: Vec::new(),
                    retryable_locations: Vec::new(),
                    has_non_retryable_failures: true,
                    message,
                });
            }
        } else {
            for (i, dst) in restored.iter().enumerate() {
                emit(OperationEvent::RestoreProgress {
                    request_id: id,
                    completed: i + 1,
                    total,
                    restored_location: Some(dst.clone()),
                });
            }
            emit(OperationEvent::Restored {
                request_id: id,
                locations: Vec::new(),
                restored,
            });
        }
    });
    LoadHandle::new(move || cancelled.store(true, AtomicOrdering::Release))
}
/// Cross-volume move on the bounded native worker: the source is deleted only
/// after the destination copy was published without clobbering and verified.
async fn move_verified_native(
    state: &Rc<ProviderState>,
    source: &Path,
    target: &Path,
    id: OperationRequestId,
    cancel: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let (sender, receiver) = futures_channel::oneshot::channel();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let callback: Rc<dyn Fn(OperationEvent)> = Rc::new(move |event| {
        let outcome = match event {
            OperationEvent::Pasted { .. } => Ok(()),
            OperationEvent::Failed { message, .. } => Err(message),
            _ => return,
        };
        if let Some(sender) = sender.borrow_mut().take() {
            let _ = sender.send(outcome);
        }
    });
    // Dropping the handle requests native cancellation; it must stay alive
    // until the worker reports back.
    let handle = Rc::new(RefCell::new(Some(state.submit(
        LocalOperation::MoveVerified {
            source: source.to_path_buf(),
            destination: target.to_path_buf(),
        },
        id,
        Success::Copy {
            source: Location::local(source.to_path_buf()),
            destination: Location::local(target.to_path_buf()),
        },
        callback,
    ))));
    let cancel_handle = handle.clone();
    let signal = cancel.connect_cancelled_local(move |_| {
        cancel_handle.borrow_mut().take();
    });
    let outcome = receiver.await;
    if let Some(signal) = signal {
        cancel.disconnect_cancelled(signal);
    }
    handle.borrow_mut().take();
    match outcome {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => Err(glib::Error::new(gio::IOErrorEnum::Failed, &message)),
        Err(_) => Err(glib::Error::new(
            gio::IOErrorEnum::Cancelled,
            "move cancelled",
        )),
    }
}

/// A cancellable native rename with no silent copy/delete fallback.
/// The flags intentionally exclude OVERWRITE.
async fn move_no_replace(
    source: &Path,
    target: &Path,
    cancel: gio::Cancellable,
) -> Result<(), glib::Error> {
    let file = gio::File::for_path(source);
    let destination = gio::File::for_path(target);
    gio::GioFuture::new(&file, move |file, _, result| {
        file.move_async(
            &destination,
            gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE,
            glib::Priority::DEFAULT,
            Some(&cancel),
            None,
            move |outcome| result.resolve(outcome),
        );
    })
    .await
}

/// Rename in-place without replacing an existing destination. GIO's native
/// move with NO_FALLBACK_FOR_MOVE performs a same-volume rename (including
/// directories) and rejects collisions; it never degrades to copy + delete.
/// The async operation does not block GTK's main loop on filesystem I/O.
/// A failure is reported as a failure, never as a successful rename.
fn async_rename_no_replace(
    source: std::path::PathBuf,
    target: std::path::PathBuf,
    request_id: OperationRequestId,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    use strata_native_io::windows_paths::{PathKind, classify, valid_child_name};

    for path in [&source, &target] {
        if !matches!(
            classify(path),
            PathKind::LocalAbsolute | PathKind::NetworkAbsolute
        ) || !path.file_name().is_some_and(valid_child_name)
        {
            return rejected(request_id, emit, "Invalid Windows rename path");
        }
    }
    if source == target {
        emit(OperationEvent::Renamed { request_id });
        return LoadHandle::new(|| {});
    }
    let cancel = gio::Cancellable::new();
    let task_cancel = cancel.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        crate::services::file_mutations::release(&[Location::local(&source)]).await;

        let result = move_no_replace(&source, &target, task_cancel).await;
        match result {
            Ok(()) => emit(OperationEvent::Renamed { request_id }),
            Err(error) if error.matches(gio::IOErrorEnum::Cancelled) => {
                // No operation was confirmed complete. Cancelling a handle
                // suppresses its completion just as the existing Windows
                // create-file and create-directory providers do.
            }
            Err(error) => emit(OperationEvent::Failed {
                request_id,
                message: crate::services::gio_error_message(&error),
                password_failure: None,
            }),
        }
    });
    LoadHandle::new(move || cancel.cancel())
}

fn async_create_directory(
    request: CreateDirectoryRequest,
    emit: Rc<dyn Fn(OperationEvent)>,
) -> LoadHandle {
    let Some(parent) = request.parent.native_path() else {
        return rejected(
            request.id,
            emit,
            "Windows new folder requires a local filesystem path",
        );
    };
    let directory = gio::File::for_path(parent);
    let cancel = gio::Cancellable::new();
    let task_cancel = cancel.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let mut suffix = 0u64;
        loop {
            if task_cancel.is_cancelled() {
                return;
            }
            let candidate = if suffix == 0 {
                request.name.clone()
            } else {
                format!("{} ({suffix})", request.name)
            };
            if !strata_native_io::windows_paths::valid_child_name(OsStr::new(&candidate)) {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: "Invalid Windows folder name".to_owned(),
                    password_failure: None,
                });
                return;
            }
            let target = directory.child(&candidate);
            let result = gio::GioFuture::new(&target, {
                let task_cancel = task_cancel.clone();
                move |file, _, result| {
                    file.make_directory_async(
                        glib::Priority::DEFAULT,
                        Some(&task_cancel),
                        move |output| result.resolve(output),
                    );
                }
            })
            .await;
            match result {
                Ok(()) => {
                    emit(OperationEvent::EntryCreated {
                        request_id: request.id,
                        location: Location::local(
                            request
                                .parent
                                .native_path()
                                .expect("validated")
                                .join(candidate),
                        ),
                    });
                    return;
                }
                Err(error) if suffix < u64::MAX && error.matches(gio::IOErrorEnum::Exists) => {
                    suffix += 1;
                }
                Err(error) => {
                    if !task_cancel.is_cancelled() {
                        emit(OperationEvent::Failed {
                            request_id: request.id,
                            message: crate::services::gio_error_message(&error),
                            password_failure: None,
                        });
                    }
                    return;
                }
            }
        }
    });
    LoadHandle::new(move || cancel.cancel())
}

fn async_create_file(request: CreateFileRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    if let Err(message) = validate_basename(&request.name) {
        return rejected(request.id, emit, crate::i18n::tr(message));
    }
    if !strata_native_io::windows_paths::valid_child_name(OsStr::new(&request.name)) {
        return rejected(request.id, emit, "Invalid Windows filename");
    }
    let Some(parent) = request.parent.native_path() else {
        return rejected(
            request.id,
            emit,
            "Windows new file requires a local filesystem path",
        );
    };
    let directory = gio::File::for_path(parent);
    let cancel = gio::Cancellable::new();
    let task_cancel = cancel.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let mut suffix = 0u64;
        loop {
            if task_cancel.is_cancelled() {
                return;
            }
            let candidate = if suffix == 0 {
                request.name.clone()
            } else {
                format!("{} ({suffix})", request.name)
            };
            if !strata_native_io::windows_paths::valid_child_name(OsStr::new(&candidate)) {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: "Invalid Windows filename".to_owned(),
                    password_failure: None,
                });
                return;
            }
            let target = directory.child(&candidate);
            let result = gio::GioFuture::new(&target, {
                let task_cancel = task_cancel.clone();
                move |file, _, result| {
                    file.create_async(
                        gio::FileCreateFlags::NONE,
                        glib::Priority::DEFAULT,
                        Some(&task_cancel),
                        move |output| result.resolve(output),
                    );
                }
            })
            .await;
            match result {
                Ok(stream) => {
                    if let Err(error) = stream.close_future(glib::Priority::DEFAULT).await {
                        emit(OperationEvent::Failed {
                            request_id: request.id,
                            message: crate::services::gio_error_message(&error),
                            password_failure: None,
                        });
                        return;
                    }
                    emit(if request.unique_name {
                        OperationEvent::EntryCreated {
                            request_id: request.id,
                            location: Location::local(
                                request
                                    .parent
                                    .native_path()
                                    .expect("validated")
                                    .join(candidate),
                            ),
                        }
                    } else {
                        OperationEvent::Created {
                            request_id: request.id,
                        }
                    });
                    return;
                }
                Err(error)
                    if request.unique_name
                        && suffix < u64::MAX
                        && error.matches(gio::IOErrorEnum::Exists) =>
                {
                    suffix += 1;
                }
                Err(error) => {
                    if !task_cancel.is_cancelled() {
                        emit(OperationEvent::Failed {
                            request_id: request.id,
                            message: crate::services::gio_error_message(&error),
                            password_failure: None,
                        });
                    }
                    return;
                }
            }
        }
    });
    LoadHandle::new(move || cancel.cancel())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Run GIO's Windows rename API to completion on the GLib main context
    /// without needing a display server or a GTK window.
    fn collect_rename(
        source: std::path::PathBuf,
        target: std::path::PathBuf,
    ) -> Vec<OperationEvent> {
        let received = Rc::new(RefCell::new(Vec::new()));
        let captured = received.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| captured.borrow_mut().push(event));
        let _handle = async_rename_no_replace(source, target, OperationRequestId(1), emit);
        let context = glib::MainContext::default();
        let started = Instant::now();
        while received.borrow().is_empty() {
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "rename timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        std::mem::take(&mut *received.borrow_mut())
    }

    #[test]
    fn native_rename_never_clobbers_an_existing_file() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("temporary folder");
        let source = root.path().join("source.txt");
        let existing = root.path().join("existing.txt");
        std::fs::write(&source, b"original").expect("source fixture");
        std::fs::write(&existing, b"must remain").expect("destination fixture");

        let events = collect_rename(source.clone(), existing.clone());
        assert!(matches!(events.as_slice(), [OperationEvent::Failed { .. }]));
        assert_eq!(
            std::fs::read(&source).expect("source retained"),
            b"original"
        );
        assert_eq!(
            std::fs::read(&existing).expect("destination retained"),
            b"must remain"
        );
    }

    #[test]
    fn native_rename_supports_folders_and_undo_without_copying() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("temporary folder");
        let source = root.path().join("before");
        let renamed = root.path().join("after");
        std::fs::create_dir(&source).expect("source directory");
        std::fs::write(source.join("payload.txt"), b"intact").expect("nested fixture");

        let events = collect_rename(source.clone(), renamed.clone());
        assert!(matches!(
            events.as_slice(),
            [OperationEvent::Renamed { .. }]
        ));
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(renamed.join("payload.txt")).expect("renamed directory"),
            b"intact"
        );
        let undone = collect_rename(renamed.clone(), source.clone());
        assert!(matches!(
            undone.as_slice(),
            [OperationEvent::Renamed { .. }]
        ));
        assert!(!renamed.exists());
        assert_eq!(
            std::fs::read(source.join("payload.txt")).expect("restored directory"),
            b"intact"
        );
    }

    fn run_verified_move(source: &Path, target: &Path) -> Result<(), glib::Error> {
        let state = Rc::new(ProviderState::default());
        let cancel = gio::Cancellable::new();
        glib::MainContext::default().block_on(move_verified_native(
            &state,
            source,
            target,
            OperationRequestId(21),
            &cancel,
        ))
    }

    #[test]
    fn verified_move_fallback_moves_a_tree_through_the_native_worker() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("temporary tree");
        let source = root.path().join("folder");
        let target = root.path().join("moved");
        std::fs::create_dir_all(source.join("nested")).expect("source tree");
        std::fs::write(source.join("nested").join("a.txt"), b"payload").expect("source file");

        run_verified_move(&source, &target).expect("verified move");

        assert!(!source.exists());
        assert_eq!(
            std::fs::read(target.join("nested").join("a.txt")).expect("moved file"),
            b"payload"
        );
    }

    #[test]
    fn verified_move_fallback_keeps_source_and_destination_on_collision() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("temporary tree");
        let source = root.path().join("a.txt");
        let target = root.path().join("b.txt");
        std::fs::write(&source, b"source").expect("source file");
        std::fs::write(&target, b"protected").expect("existing destination");

        assert!(run_verified_move(&source, &target).is_err());

        assert_eq!(std::fs::read(&source).expect("source kept"), b"source");
        assert_eq!(
            std::fs::read(&target).expect("destination kept"),
            b"protected"
        );
    }
    #[test]
    fn native_move_batch_reports_partial_completion_without_overwriting() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("temporary tree");
        let originals = root.path().join("originals");
        let target = root.path().join("target");
        std::fs::create_dir(&originals).expect("source folder");
        std::fs::create_dir(&target).expect("destination folder");
        let first = originals.join("first.txt");
        let second = originals.join("second.txt");
        std::fs::write(&first, b"one").expect("first source");
        std::fs::write(&second, b"two").expect("second source");
        std::fs::write(target.join("second.txt"), b"protected").expect("concurrent destination");
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| captured.borrow_mut().push(event));
        let _handle = async_move_paths(
            None,
            OperationRequestId(12),
            vec![
                (
                    Location::local(first.clone()),
                    first.clone(),
                    target.join("first.txt"),
                ),
                (
                    Location::local(second.clone()),
                    second.clone(),
                    target.join("second.txt"),
                ),
            ],
            Vec::new(),
            emit,
        );
        let context = glib::MainContext::default();
        let started = Instant::now();
        loop {
            while context.pending() {
                context.iteration(false);
            }
            if events
                .borrow()
                .iter()
                .any(|event| matches!(event, OperationEvent::TransferFailed { .. }))
            {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "batch move timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!first.exists());
        assert_eq!(
            std::fs::read(target.join("first.txt")).expect("first moved"),
            b"one"
        );
        assert_eq!(std::fs::read(&second).expect("second preserved"), b"two");
        assert_eq!(
            std::fs::read(target.join("second.txt")).expect("protected destination"),
            b"protected"
        );
        assert!(events.borrow().iter().any(|event| matches!(
            event,
            OperationEvent::TransferFailed {
                completed_locations,
                ..
            } if completed_locations == &vec![Location::local(first.clone())]
        )));
    }

    #[test]
    fn native_batch_copy_reports_created_paths_and_original_sources() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("temporary tree");
        let dest = root.path().join("copied");
        std::fs::create_dir(&dest).expect("destination");
        let first = root.path().join("first.txt");
        let second = root.path().join("second.txt");
        std::fs::write(&first, b"alpha").expect("first fixture");
        std::fs::write(&second, b"beta").expect("second fixture");
        let sources = vec![
            Location::local(first.clone()),
            Location::local(second.clone()),
        ];
        let request = PasteRequest {
            id: OperationRequestId(50),
            destination: Location::local(dest.clone()),
            items: sources
                .iter()
                .cloned()
                .map(|source| crate::services::PasteItem {
                    source,
                    conflict: TransferConflict::FailIfExists,
                })
                .collect(),
            move_sources: false,
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| captured.borrow_mut().push(event));
        let provider = LocalOperationProvider::default();
        let _handle = provider.paste(request, emit);
        let context = glib::MainContext::default();
        let started = Instant::now();
        loop {
            while context.pending() {
                context.iteration(false);
            }
            if events
                .borrow()
                .iter()
                .any(|event| matches!(event, OperationEvent::Pasted { .. }))
            {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "batch copy timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            std::fs::read(dest.join("first.txt")).expect("first copy"),
            b"alpha"
        );
        assert_eq!(
            std::fs::read(dest.join("second.txt")).expect("second copy"),
            b"beta"
        );
        assert_eq!(std::fs::read(&first).expect("original first"), b"alpha");
        assert_eq!(std::fs::read(&second).expect("original second"), b"beta");
        let observed = events.borrow();
        assert_eq!(
            observed
                .iter()
                .filter(|event| matches!(
                    event,
                    OperationEvent::TransferProgress {
                        created_location: Some(_),
                        ..
                    }
                ))
                .count(),
            2,
            "every completed file must enter the browser's undo ledger"
        );
        assert!(observed.iter().any(|event| matches!(
            event,
            OperationEvent::Pasted { locations, .. } if locations == &sources
        )));
    }

    #[test]
    fn cancelled_operations_keep_polling_until_the_native_queue_is_drained() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("temporary destination");
        let state = Rc::new(ProviderState::default());
        let received = Rc::new(RefCell::new(Vec::new()));
        let captured = received.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| captured.borrow_mut().push(event));
        let context = glib::MainContext::default();

        // More results than the native completion channel can buffer (64).
        // Cancelling a LoadHandle must leave its entry in the map until the
        // completion is consumed; otherwise polling stops and workers block.
        for batch in 0..6 {
            let handles: Vec<_> = (0..16)
                .map(|item| {
                    state.submit(
                        LocalOperation::CreateDirectory(
                            root.path().join(format!("cancel-{batch}-{item}")),
                        ),
                        OperationRequestId(batch * 16 + item),
                        Success::Directory,
                        emit.clone(),
                    )
                })
                .collect();
            assert_eq!(state.pending.borrow().len(), 16);
            drop(handles);
            assert_eq!(
                state.pending.borrow().len(),
                16,
                "dropping handles must not discard unconsumed completions",
            );

            let started = Instant::now();
            while !state.pending.borrow().is_empty() {
                while context.pending() {
                    context.iteration(false);
                }
                assert!(
                    started.elapsed() < Duration::from_secs(10),
                    "cancelled completions were not drained",
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(!state.polling.get(), "poller should stop after draining");
        }

        // A new request must still finish after the cancellation storm.
        let destination = root.path().join("still-works");
        let _handle = state.submit(
            LocalOperation::CreateDirectory(destination.clone()),
            OperationRequestId(1000),
            Success::Directory,
            emit.clone(),
        );
        let started = Instant::now();
        while !state.pending.borrow().is_empty() {
            while context.pending() {
                context.iteration(false);
            }
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(destination.is_dir());
        assert!(received.borrow().iter().any(|event| matches!(
            event,
            OperationEvent::Created {
                request_id: OperationRequestId(1000)
            }
        )));
    }

    #[test]
    fn native_recycle_and_restore_cycle_recovers_files() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("tempdir");
        let file = root.path().join("recycle-me.txt");
        std::fs::write(&file, b"content to be recycled").expect("write file");
        let loc = Location::local(file.clone());

        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| captured.borrow_mut().push(event));

        let _handle = recycle_locations(OperationRequestId(200), vec![loc.clone()], emit.clone());

        let context = glib::MainContext::default();
        let started = Instant::now();
        while !events
            .borrow()
            .iter()
            .any(|e| matches!(e, OperationEvent::Deleted { .. }))
        {
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "delete timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(!file.exists(), "file should be moved to recycle bin");

        events.borrow_mut().clear();
        let _restore_handle = restore_locations(OperationRequestId(201), vec![loc.clone()], emit);

        let started = Instant::now();
        while !events
            .borrow()
            .iter()
            .any(|e| matches!(e, OperationEvent::Restored { .. }))
        {
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "restore timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(file.exists(), "file should be restored");
        assert_eq!(
            std::fs::read(&file).expect("read restored file"),
            b"content to be recycled"
        );
    }

    #[test]
    fn native_paste_replace_existing_and_undo_merge() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("tempdir");
        let source_dir = root.path().join("source");
        let dest_dir = root.path().join("dest");
        std::fs::create_dir_all(&source_dir).expect("create source");
        std::fs::create_dir_all(&dest_dir).expect("create dest");

        let source_file = source_dir.join("file.txt");
        let dest_file = dest_dir.join("file.txt");
        std::fs::write(&source_file, b"incoming content").expect("write source");
        std::fs::write(&dest_file, b"original content").expect("write dest");

        let provider = LocalOperationProvider::default();
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| captured.borrow_mut().push(event));

        let _handle = provider.paste(
            PasteRequest {
                id: OperationRequestId(300),
                destination: Location::local(dest_dir.clone()),
                items: vec![crate::services::PasteItem {
                    source: Location::local(source_file.clone()),
                    conflict: TransferConflict::ReplaceExisting,
                }],
                move_sources: false,
            },
            emit.clone(),
        );

        let context = glib::MainContext::default();
        let started = Instant::now();
        while !events
            .borrow()
            .iter()
            .any(|e| matches!(e, OperationEvent::Pasted { .. }))
        {
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "paste timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        assert_eq!(
            std::fs::read(&dest_file).expect("read replaced file"),
            b"incoming content"
        );

        // Find the Merged event to verify overwritten path
        let merged_event = events
            .borrow()
            .iter()
            .find(|e| matches!(e, OperationEvent::Merged { .. }))
            .cloned();
        let Some(OperationEvent::Merged { overwritten, .. }) = merged_event else {
            panic!("Expected OperationEvent::Merged event");
        };
        assert_eq!(overwritten, vec![Location::local(dest_file.clone())]);

        // Now undo the merge: original should be restored
        events.borrow_mut().clear();
        let _undo_handle = provider.undo_merge(
            UndoMergeRequest {
                id: OperationRequestId(301),
                created: Vec::new(),
                overwritten: vec![Location::local(dest_file.clone())],
                originals: HashMap::new(),
            },
            emit,
        );

        let started = Instant::now();
        while !events
            .borrow()
            .iter()
            .any(|e| matches!(e, OperationEvent::Restored { .. }))
        {
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "undo_merge timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        assert_eq!(
            std::fs::read(&dest_file).expect("read restored file"),
            b"original content"
        );
    }

    #[test]
    fn native_paste_merge_directory_and_undo_merge() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("tempdir");
        let source_dir = root.path().join("source");
        let dest_dir = root.path().join("dest");
        std::fs::create_dir_all(source_dir.join("folder")).expect("create source/folder");
        std::fs::create_dir_all(dest_dir.join("folder")).expect("create dest/folder");

        let source_new = source_dir.join("folder").join("new.txt");
        let source_shared = source_dir.join("folder").join("shared.txt");
        let dest_existing = dest_dir.join("folder").join("existing.txt");
        let dest_shared = dest_dir.join("folder").join("shared.txt");
        let dest_new = dest_dir.join("folder").join("new.txt");

        std::fs::write(&source_new, b"new from source").expect("write source_new");
        std::fs::write(&source_shared, b"shared incoming").expect("write source_shared");
        std::fs::write(&dest_existing, b"dest existing").expect("write dest_existing");
        std::fs::write(&dest_shared, b"dest shared original").expect("write dest_shared");

        let provider = LocalOperationProvider::default();
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| captured.borrow_mut().push(event));

        let _handle = provider.paste(
            PasteRequest {
                id: OperationRequestId(400),
                destination: Location::local(dest_dir.clone()),
                items: vec![crate::services::PasteItem {
                    source: Location::local(source_dir.join("folder")),
                    conflict: TransferConflict::Merge,
                }],
                move_sources: false,
            },
            emit.clone(),
        );

        let context = glib::MainContext::default();
        let started = Instant::now();
        while !events
            .borrow()
            .iter()
            .any(|e| matches!(e, OperationEvent::Pasted { .. }))
        {
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "merge paste timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        // Verify merge results
        assert_eq!(
            std::fs::read(&dest_existing).expect("existing kept"),
            b"dest existing"
        );
        assert_eq!(
            std::fs::read(&dest_new).expect("new file created"),
            b"new from source"
        );
        assert_eq!(
            std::fs::read(&dest_shared).expect("shared overwritten"),
            b"shared incoming"
        );

        // Find Merged event
        let merged_event = events
            .borrow()
            .iter()
            .find(|e| matches!(e, OperationEvent::Merged { .. }))
            .cloned();
        let Some(OperationEvent::Merged {
            created,
            overwritten,
            ..
        }) = merged_event
        else {
            panic!("Expected OperationEvent::Merged");
        };
        assert_eq!(created, vec![Location::local(dest_new.clone())]);
        assert_eq!(overwritten, vec![Location::local(dest_shared.clone())]);

        // Undo merge
        events.borrow_mut().clear();
        let _undo_handle = provider.undo_merge(
            UndoMergeRequest {
                id: OperationRequestId(401),
                created: vec![Location::local(dest_new.clone())],
                overwritten: vec![Location::local(dest_shared.clone())],
                originals: HashMap::new(),
            },
            emit,
        );

        let started = Instant::now();
        while !events
            .borrow()
            .iter()
            .any(|e| matches!(e, OperationEvent::Restored { .. }))
        {
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "undo_merge timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        // Verify pre-merge state is restored
        assert_eq!(
            std::fs::read(&dest_shared).expect("shared restored"),
            b"dest shared original"
        );
        assert!(!dest_new.exists(), "new file should be removed by undo");
        assert_eq!(
            std::fs::read(&dest_existing).expect("existing untouched"),
            b"dest existing"
        );
    }

    #[test]
    fn native_move_keep_both_auto_numbers_without_overwriting() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("tempdir");
        let source_dir = root.path().join("source");
        let dest_dir = root.path().join("dest");
        std::fs::create_dir_all(&source_dir).expect("create source");
        std::fs::create_dir_all(&dest_dir).expect("create dest");

        let source_file = source_dir.join("doc.txt");
        let dest_file = dest_dir.join("doc.txt");
        let expected_numbered = dest_dir.join("doc (1).txt");

        std::fs::write(&source_file, b"moved content").expect("write source");
        std::fs::write(&dest_file, b"original content").expect("write dest");

        let provider = LocalOperationProvider::default();
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| captured.borrow_mut().push(event));

        let _handle = provider.paste(
            PasteRequest {
                id: OperationRequestId(500),
                destination: Location::local(dest_dir.clone()),
                items: vec![crate::services::PasteItem {
                    source: Location::local(source_file.clone()),
                    conflict: TransferConflict::KeepBoth,
                }],
                move_sources: true,
            },
            emit,
        );

        let context = glib::MainContext::default();
        let started = Instant::now();
        while !events
            .borrow()
            .iter()
            .any(|e| matches!(e, OperationEvent::Pasted { .. }))
        {
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "move timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        assert_eq!(
            std::fs::read(&dest_file).expect("original untouched"),
            b"original content"
        );
        assert_eq!(
            std::fs::read(&expected_numbered).expect("numbered candidate created"),
            b"moved content"
        );
        assert!(!source_file.exists(), "source should be moved");
    }

    #[test]
    fn native_undo_move_cleans_up_folders() {
        let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
            .lock()
            .expect("serialize main context tests");
        let root = tempfile::tempdir().expect("tempdir");
        let source_file = root.path().join("source.txt");
        let moved_file = root.path().join("moved.txt");
        let cleanup_folder = root.path().join("cleanup_folder");

        std::fs::write(&moved_file, b"content").expect("write moved");
        std::fs::create_dir_all(&cleanup_folder).expect("create cleanup folder");

        let provider = LocalOperationProvider::default();
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        let emit: Rc<dyn Fn(OperationEvent)> =
            Rc::new(move |event| captured.borrow_mut().push(event));

        let _handle = provider.undo_move(
            UndoMoveRequest {
                id: OperationRequestId(600),
                items: vec![crate::services::UndoMoveItem {
                    record: crate::services::MoveRecord {
                        original: Location::local(source_file.clone()),
                        current: Location::local(moved_file.clone()),
                    },
                    conflict: TransferConflict::FailIfExists,
                }],
                cleanup_locations: vec![Location::local(cleanup_folder.clone())],
            },
            emit,
        );

        let context = glib::MainContext::default();
        let started = Instant::now();
        while !events
            .borrow()
            .iter()
            .any(|e| matches!(e, OperationEvent::Pasted { .. }))
        {
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "undo_move timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(source_file.exists(), "file should be moved back");
        assert!(!moved_file.exists(), "moved file should be gone");
        assert!(
            !cleanup_folder.exists(),
            "cleanup folder should be recycled"
        );
    }
}
