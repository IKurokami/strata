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
    Copy(Location),
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
                        Success::Copy(location) => OperationEvent::Pasted {
                            request_id,
                            locations: vec![location],
                        },
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
        if request.move_sources || request.items.len() != 1 {
            return rejected(
                request.id,
                emit,
                "Windows batch move/merge is not available yet",
            );
        }
        let item = &request.items[0];
        if item.conflict != TransferConflict::FailIfExists {
            return rejected(
                request.id,
                emit,
                "Windows overwrite, merge and Keep Both are not available yet",
            );
        }
        let (Some(source), Some(destination_dir)) =
            (item.source.native_path(), request.destination.native_path())
        else {
            return rejected(
                request.id,
                emit,
                "Windows file copy requires local filesystem paths",
            );
        };
        let Some(name) = source.file_name() else {
            return rejected(request.id, emit, "Source file has no ordinary filename");
        };
        if !strata_native_io::windows_paths::valid_child_name(name) {
            return rejected(request.id, emit, "Invalid Windows filename");
        }
        let destination = destination_dir.join(name);
        self.state.submit(
            LocalOperation::CopyFile {
                source: source.to_path_buf(),
                destination: destination.clone(),
            },
            request.id,
            Success::Copy(Location::local(destination)),
            emit,
        )
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
        rejected(request.id, emit, "Windows undo move is not implemented")
    }
    fn undo_rename(
        &self,
        request: UndoRenameRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle {
        let (Some(source), Some(target)) =
            (request.current.native_path(), request.original.native_path())
        else {
            return rejected(request.id, emit, "Windows undo rename requires local paths");
        };
        if source.parent().is_none() || source.parent() != target.parent() {
            return rejected(request.id, emit, "Undo rename must stay in the original folder");
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
    if source == target {
        emit(OperationEvent::Renamed { request_id });
        return LoadHandle::new(|| {});
    }
    let file = gio::File::for_path(&source);
    let destination = gio::File::for_path(&target);
    let cancel = gio::Cancellable::new();
    let task_cancel = cancel.clone();
    let _task = glib::MainContext::default().spawn_local(async move {
        let result = gio::GioFuture::new(&file, move |file, _, result| {
            file.move_async(
                &destination,
                gio::FileCopyFlags::NO_FALLBACK_FOR_MOVE,
                glib::Priority::DEFAULT,
                Some(&task_cancel),
                None,
                move |outcome| result.resolve(outcome),
            );
        })
        .await;
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
