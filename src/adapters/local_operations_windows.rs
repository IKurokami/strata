// SPDX-License-Identifier: MIT

//! Windows GTK operation-provider bridge. Native file mutations are submitted
//! to the bounded off-main-thread worker service; GTK only polls completions.
//! No shell, volume query, ACL preflight or UAC prompt runs on the UI thread.
//!
//! Only operations with deliberate native no-clobber semantics are enabled.
//! Recycle Bin, archive mutation, merge, move and overwrite fail explicitly
//! until their Windows-safe undo and containment contracts are implemented.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::OsStr,
    io,
    path::Path,
    rc::{Rc, Weak},
    time::Duration,
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
pub(crate) use listing::ArchiveListing;
pub(crate) use listing::{
    ARCHIVE_PREVIEW_FAILED_MESSAGE, ARCHIVE_TOO_LARGE_MESSAGE, ARCHIVE_UNSUPPORTED_MESSAGE,
    ArchiveListingStatus, INVALID_ARCHIVE, MAX_ARCHIVE_PASSWORD_BYTES, archive_payload_valid,
    decode_archive_listing, encode_archive_result,
};

pub(crate) const PASSWORD_REQUIRED: &str = "A password is required to extract this archive.";
pub(crate) const MAYBE_BAD_PASSWORD: &str = "The password may be incorrect.";

/// Archive previews must never parse untrusted files without a Windows sandbox.
pub(crate) fn list_archive_entries_direct(
    _path: &Path,
    _format: crate::services::ArchiveFormat,
    _password: Option<&str>,
    _cancelled: &std::sync::atomic::AtomicBool,
) -> Result<listing::ArchiveListing, String> {
    Err(ARCHIVE_UNSUPPORTED_MESSAGE.to_owned())
}

/// Linux fsync and removable-device flush do not map to Windows volume cache
/// semantics. Do not claim that Windows removable media is safe to eject.
pub(crate) fn flush_filesystem(_root: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native Windows removable-device flush is not implemented",
    ))
}

enum Success {
    Directory,
    Copy { source: Location, destination: Location },
}
/// Collect a bounded worker batch into one truthful terminal event.
/// Successful sources are ordered by the user's request, not worker timing.
struct BatchCopyState {
    remaining: usize,
    completed: Vec<(usize, Location)>,
    error: Option<String>,
}
impl BatchCopyState {
    fn terminal(&mut self, request_id: OperationRequestId) -> Option<OperationEvent> {
        if self.remaining != 0 {
            return None;
        }
        self.completed.sort_by_key(|(index, _)| *index);
        let completed = self
            .completed
            .iter()
            .map(|(_, location)| location.clone())
            .collect();
        Some(if let Some(message) = &self.error {
            OperationEvent::TransferFailed {
                request_id,
                completed_locations: completed,
                message: message.clone(),
            }
        } else {
            OperationEvent::Pasted {
                request_id,
                locations: completed,
            }
        })
    }
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
    polling: Cell<bool>,
}
impl ProviderState {
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
        loop {
            let next = self
                .service
                .borrow()
                .as_ref()
                .and_then(|service| service.try_recv().ok());
            let Some(OperationCompletion { id, result }) = next else {
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
            return async_move_items(request, emit);
        }
        use strata_native_io::windows_paths::{PathKind, classify, valid_child_name};

        let Some(destination_dir) = request.destination.native_path() else {
            return rejected(request.id, emit, "Windows copy requires a local destination");
        };
        if !matches!(
            classify(destination_dir),
            PathKind::LocalAbsolute | PathKind::NetworkAbsolute
        ) {
            return rejected(request.id, emit, "Invalid Windows destination");
        }
        let mut operations = Vec::with_capacity(request.items.len());
        for item in &request.items {
            if item.conflict != TransferConflict::FailIfExists {
                return rejected(
                    request.id,
                    emit,
                    "Windows overwrite, merge and Keep Both are not available yet",
                );
            }
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
            operations.push((item.source.clone(), source.to_path_buf(), destination));
        }
        // No inline filesystem calls. Each native request goes through the
        // existing bounded mutation service, with atomic no-clobber publish.
        // Aggregate multiple out-of-order completions into one terminal event.
        let total = operations.len();
        if total == 0 {
            emit(OperationEvent::Pasted {
                request_id: request.id,
                locations: Vec::new(),
            });
            return LoadHandle::new(|| {});
        }
        let batch = Rc::new(RefCell::new(BatchCopyState {
            remaining: total,
            completed: Vec::with_capacity(total),
            error: None,
        }));
        let mut handles = Vec::with_capacity(total);
        for (index, (source, path, destination)) in operations.into_iter().enumerate() {
            let progress = batch.clone();
            let outer = emit.clone();
            let callback: Rc<dyn Fn(OperationEvent)> = Rc::new(move |event| {
                // Release the RefCell borrow before calling the browser: it
                // can start another transfer synchronously from an event.
                let mut forwarded = None;
                let terminal = {
                    let mut state = progress.borrow_mut();
                    match event {
                        OperationEvent::TransferProgress {
                            request_id,
                            created_location,
                            ..
                        } => {
                            forwarded = Some(OperationEvent::TransferProgress {
                                request_id,
                                completed_items: state.completed.len() + 1,
                                completed_files: state.completed.len() + 1,
                                total_files: Some(total),
                                current_file: None,
                                transferred_bytes: 0,
                                total_bytes: None,
                                created_location,
                            });
                            None
                        }
                        OperationEvent::Pasted { locations, .. } => {
                            if let Some(location) = locations.into_iter().next() {
                                state.completed.push((index, location));
                            }
                            state.remaining -= 1;
                            state.terminal(request.id)
                        }
                        OperationEvent::Failed { message, .. } => {
                            if state.error.is_none() {
                                state.error = Some(message);
                            }
                            state.remaining -= 1;
                            state.terminal(request.id)
                        }
                        _ => None,
                    }
                };
                if let Some(event) = forwarded {
                    outer(event);
                }
                if let Some(event) = terminal {
                    outer(event);
                }
            });
            handles.push(self.state.submit(
                LocalOperation::CopyFile {
                    source: path,
                    destination: destination.clone(),
                },
                request.id,
                Success::Copy {
                    source,
                    destination: Location::local(destination),
                },
                callback,
            ));
        }
        LoadHandle::new(move || drop(handles))
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
        // Undo that requires trashing merge-created folders must wait for a
        // verified Recycle Bin adapter; never destroy those folders silently.
        if !request.cleanup_locations.is_empty() {
            return rejected(
                request.id,
                emit,
                "Windows undo merge cleanup requires Recycle Bin integration",
            );
        }
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
        async_move_paths(request.id, operations, emit)
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
        rejected(
            request.id,
            emit,
            "Windows undo copy requires Recycle Bin integration",
        )
    }
    fn undo_merge(
        &self,
        request: UndoMergeRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        rejected(
            request.id,
            emit,
            "Windows undo merge requires Recycle Bin integration",
        )
    }
    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        rejected(
            request.id,
            emit,
            "Windows Recycle Bin and permanent deletion are not yet supported",
        )
    }
    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        rejected(
            request.id,
            emit,
            "Windows Recycle Bin restore is not yet supported",
        )
    }
    fn compress(&self, request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        rejected(
            request.id,
            emit,
            "Windows archive compression needs a secure native writer",
        )
    }
    fn extract(&self, request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        rejected(
            request.id,
            emit,
            "Windows archive extraction needs a sandboxed decoder",
        )
    }
}

/// All transfer paths are validated before the first mutation. A batch
/// reports exactly which source items moved if a later item fails. Only native
/// same-volume renames are attempted: NO_FALLBACK_FOR_MOVE must not degrade
/// into non-atomic copy/delete, particularly for directories or remote drives.
fn async_move_items(request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let Some(destination_dir) = request.destination.native_path() else {
        return rejected(
            request.id,
            emit,
            "Windows move requires a local destination",
        );
    };
    if request
        .items
        .iter()
        .any(|item| item.conflict != TransferConflict::FailIfExists)
    {
        return rejected(
            request.id,
            emit,
            "Windows move conflict resolution is not implemented",
        );
    }
    let mut operations = Vec::with_capacity(request.items.len());
    for item in &request.items {
        let Some(source) = item.source.native_path() else {
            return rejected(request.id, emit, "Windows move requires local source paths");
        };
        let Some(name) = source.file_name() else {
            return rejected(request.id, emit, "Cannot move a volume root");
        };
        if !strata_native_io::windows_paths::valid_child_name(name) {
            return rejected(request.id, emit, "Invalid Windows filename");
        }
        let target = destination_dir.join(name);
        operations.push((item.source.clone(), source.to_path_buf(), target));
    }
    async_move_paths(request.id, operations, emit)
}

/// A single validated executor is shared by move and undo-move, keeping
/// failure/cancellation accounting consistent across both directions.
fn async_move_paths(
    id: OperationRequestId,
    operations: Vec<(Location, std::path::PathBuf, std::path::PathBuf)>,
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
                move_no_replace(source, target, task_cancel.clone()).await
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
        emit(OperationEvent::Pasted {
            request_id: id,
            locations: completed,
        });
    });
    LoadHandle::new(move || cancel.cancel())
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
}
