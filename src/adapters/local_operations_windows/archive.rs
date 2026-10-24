// SPDX-License-Identifier: MIT

//! Windows native archive compression and extraction.
//!
//! Provides safe in-process or background extraction and compression with:
//! - Traversal protection (strict rejection of `..`, drive letters, UNC, DOS reserved devices, alternate data streams).
//! - Non-clobbering Keep Both extraction naming (`name (1)`, `name (2)`, etc.).
//! - Cancellation via `LoadHandle` and `Arc<AtomicBool>`.
//! - Progress reporting via 100ms timer.
//! - Support for ZIP, TAR, TAR.GZ, and 7Z formats.

use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use gtk::{gio, glib};

use crate::{
    model::Location,
    services::{
        ArchiveFormat, CancelledOperation, CompressRequest, ExtractRequest, LoadHandle,
        OperationEvent, OperationRequestId, PasswordFailure, TransferConflict, validate_basename,
    },
};

pub(crate) const PASSWORD_REQUIRED: &str = "A password is required to extract this archive.";
pub(crate) const MAYBE_BAD_PASSWORD: &str = "The password may be incorrect.";
const ARCHIVE_CANCELLED: &str = "Operation cancelled";
#[cfg(feature = "rar")]
#[path = "archive/rar.rs"]
mod rar;

#[cfg(test)]
#[path = "archive/folder_tests.rs"]
mod folder_tests;

const COPY_BUF_SIZE: usize = 1024 * 1024; // 1 MiB

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ArchiveError {
    Cancelled,
    Failed(String),
    PasswordRequired(String),
    IncorrectPassword(String),
}

impl ArchiveError {
    pub(crate) fn user_message(&self) -> String {
        match self {
            Self::Cancelled => crate::i18n::tr(ARCHIVE_CANCELLED),
            Self::PasswordRequired(msg) | Self::IncorrectPassword(msg) => crate::i18n::tr(msg),
            Self::Failed(msg) => msg.clone(),
        }
    }

    pub(crate) fn password_failure(&self) -> Option<PasswordFailure> {
        match self {
            Self::PasswordRequired(_) => Some(PasswordFailure::Required),
            Self::IncorrectPassword(_) => Some(PasswordFailure::Incorrect),
            Self::Cancelled | Self::Failed(_) => None,
        }
    }
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.user_message())
    }
}

impl std::error::Error for ArchiveError {}

impl From<String> for ArchiveError {
    fn from(s: String) -> Self {
        Self::Failed(s)
    }
}

impl From<&str> for ArchiveError {
    fn from(s: &str) -> Self {
        Self::Failed(s.to_string())
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), ArchiveError> {
    if cancelled.load(Ordering::Relaxed) {
        Err(ArchiveError::Cancelled)
    } else {
        Ok(())
    }
}

/// Sanitizes archive member paths to prevent Zip Slip / Tar Slip directory traversal
/// and Windows-specific attack vectors.
pub(crate) fn sanitize_archive_member_path(name: &str) -> Result<PathBuf, String> {
    let normalized = name.replace('\\', "/");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return Err("Refusing empty archive path".to_string());
    }

    // Reject absolute paths or UNC network paths
    if trimmed.starts_with('/') || trimmed.starts_with("//") {
        return Err(format!("Refusing absolute or UNC archive path: {name}"));
    }

    // Reject drive-qualified paths (e.g. C:, D:)
    if trimmed.len() >= 2 {
        let first = trimmed.as_bytes()[0];
        let second = trimmed.as_bytes()[1];
        if (first.is_ascii_alphabetic() && second == b':') || second == b':' {
            return Err(format!("Refusing drive-qualified archive path: {name}"));
        }
    }

    // Reject NTFS alternate data streams
    if trimmed.contains(':') {
        return Err(format!(
            "Refusing archive path containing alternate data streams: {name}"
        ));
    }

    let mut clean_path = PathBuf::new();
    for component in trimmed.split('/') {
        let comp = component.trim();
        match comp {
            "" | "." => {}
            ".." => {
                if !clean_path.pop() {
                    return Err(format!("Refusing path traversal in archive member: {name}"));
                }
            }
            part => {
                let stem = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
                if matches!(
                    stem.as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                ) {
                    return Err(format!("Refusing reserved Windows device name: {name}"));
                }
                clean_path.push(part);
            }
        }
    }

    if clean_path.as_os_str().is_empty() {
        return Err("Refusing empty archive path".to_string());
    }

    Ok(clean_path)
}

/// Resolves conflicting top-level names during extraction using Keep Both numbering.
#[derive(Default)]
pub(crate) struct WindowsExtractResolver {
    renames: HashMap<std::ffi::OsString, std::ffi::OsString>,
}

impl WindowsExtractResolver {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn resolve(
        &mut self,
        destination: &Path,
        rel_path: &Path,
    ) -> Result<PathBuf, String> {
        let top = rel_path
            .components()
            .next()
            .and_then(|c| match c {
                std::path::Component::Normal(name) => Some(name),
                _ => None,
            })
            .ok_or_else(|| "Archive entry has no file name".to_string())?;

        let top_renamed = if let Some(existing) = self.renames.get(top) {
            existing.clone()
        } else {
            let available = find_available_name(destination, top)?;
            self.renames.insert(top.to_os_string(), available.clone());
            available
        };

        let mut out = PathBuf::from(top_renamed);
        for comp in rel_path.components().skip(1) {
            out.push(comp);
        }
        Ok(out)
    }
}

pub(crate) fn find_available_name(
    destination: &Path,
    name: &std::ffi::OsStr,
) -> Result<std::ffi::OsString, String> {
    let initial_path = destination.join(name);
    if !initial_path.exists() {
        return Ok(name.to_os_string());
    }

    let is_file = initial_path.is_file();
    let name_path = Path::new(name);
    let stem = name_path.file_stem().unwrap_or(name).to_string_lossy();
    let ext = name_path.extension().map(|e| e.to_string_lossy());

    for idx in 1..=10_000 {
        let candidate_str = match &ext {
            Some(extension) if is_file => format!("{stem} ({idx}).{extension}"),
            _ => format!("{stem} ({idx})"),
        };
        let candidate_path = destination.join(&candidate_str);
        if !candidate_path.exists() {
            return Ok(std::ffi::OsString::from(candidate_str));
        }
    }
    Err(format!(
        "Could not allocate a unique name for {}",
        name.to_string_lossy()
    ))
}

fn archive_progress_timer(
    request_id: OperationRequestId,
    progress: &Arc<AtomicUsize>,
    total: &Arc<AtomicUsize>,
    cancelled: &Arc<AtomicBool>,
    emit: &Rc<dyn Fn(OperationEvent)>,
) -> glib::SourceId {
    let timer_progress = progress.clone();
    let timer_total = total.clone();
    let timer_cancelled = cancelled.clone();
    let timer_emit = emit.clone();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if !timer_cancelled.load(Ordering::Relaxed) {
            timer_emit(OperationEvent::ArchiveProgress {
                request_id,
                completed: timer_progress.load(Ordering::Relaxed),
                total: timer_total.load(Ordering::Relaxed),
            });
        }
        glib::ControlFlow::Continue
    })
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

pub(crate) fn extract(request: ExtractRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancelled = Arc::new(AtomicBool::new(false));
    let task_cancelled = cancelled.clone();
    let work_cancelled = cancelled.clone();
    let destination = request.destination.clone();

    let _task = glib::MainContext::default().spawn_local(async move {
        let Some(archive_path) = request.entry.location.native_path().map(Path::to_path_buf) else {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: crate::i18n::tr("Archive must be a local file"),
                password_failure: None,
            });
            return;
        };

        if let Ok(metadata) = std::fs::metadata(&archive_path)
            && !metadata.is_file()
        {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: rust_i18n::t!(
                    "Not an archive: “%{name}”",
                    name = request.entry.display_name
                )
                .into_owned(),
                password_failure: None,
            });
            return;
        }

        let Some(dest_dir) = request.destination.native_path().map(Path::to_path_buf) else {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: crate::i18n::tr("Extract destination must be a local path"),
                password_failure: None,
            });
            return;
        };

        let created_dest = request.created_destination || !dest_dir.exists();
        if created_dest && let Err(e) = std::fs::create_dir_all(&dest_dir) {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: rust_i18n::t!(
                    "Could not create folder: %{error}",
                    error = crate::services::io_error_detail(&e)
                )
                .into_owned(),
                password_failure: None,
            });
            return;
        }

        let dest_dir_for_cleanup = dest_dir.clone();
        let format = ArchiveFormat::from_extension(&request.entry.display_name);
        let password = request.password.clone();
        let display_name = request.entry.display_name.clone();
        let into_folder = request.into_folder;
        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));

        emit(OperationEvent::ArchiveStarted {
            request_id: request.id,
            total: 0,
        });

        let timer_id =
            archive_progress_timer(request.id, &progress, &total, &task_cancelled, &emit);
        let work_progress = progress.clone();
        let work_total = total.clone();

        let result = gio::spawn_blocking(move || {
            let decode = |dest_dir: &Path| match format {
                Some(ArchiveFormat::Zip) => extract_zip(
                    &archive_path,
                    dest_dir,
                    password.as_deref(),
                    &work_progress,
                    &work_total,
                    &work_cancelled,
                ),
                Some(ArchiveFormat::Tar) => extract_tar(
                    &archive_path,
                    dest_dir,
                    false,
                    &work_progress,
                    &work_total,
                    &work_cancelled,
                ),
                Some(ArchiveFormat::TarGz) => extract_tar(
                    &archive_path,
                    dest_dir,
                    true,
                    &work_progress,
                    &work_total,
                    &work_cancelled,
                ),
                Some(ArchiveFormat::SevenZ) => extract_sevenz(
                    &archive_path,
                    dest_dir,
                    password.as_deref(),
                    &work_progress,
                    &work_total,
                    &work_cancelled,
                ),
                #[cfg(feature = "rar")]
                Some(ArchiveFormat::Rar) => rar::extract_rar(
                    &archive_path,
                    dest_dir,
                    password.as_deref(),
                    &work_progress,
                    &work_cancelled,
                ),
                _ => Err(ArchiveError::Failed(
                    rust_i18n::t!("Unsupported archive format: %{name}", name = display_name)
                        .into_owned(),
                )),
            };
            if into_folder {
                extract_named_folder(&dest_dir, &display_name, &work_cancelled, decode)
            } else {
                decode(&dest_dir)
            }
        })
        .await;

        timer_id.remove();

        if created_dest
            && !matches!(result, Ok(Ok(_)))
            && std::fs::read_dir(&dest_dir_for_cleanup)
                .is_ok_and(|mut entries| entries.next().is_none())
        {
            let _ = std::fs::remove_dir(&dest_dir_for_cleanup);
        }

        match result {
            Ok(Ok(first_name)) => {
                let created = first_name
                    .iter()
                    .filter_map(|name| destination.child(OsStr::new(name)))
                    .collect();
                emit(OperationEvent::Extracted {
                    request_id: request.id,
                    first_name,
                    created,
                });
            }
            Ok(Err(ArchiveError::Cancelled)) => {
                emit(OperationEvent::Cancelled {
                    request_id: request.id,
                    result: CancelledOperation {
                        completed: Vec::new(),
                        failed: Vec::new(),
                        not_attempted: Vec::new(),
                        affected_locations: std::collections::HashSet::from([destination]),
                    },
                });
            }
            Ok(Err(error)) => {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    password_failure: error.password_failure(),
                    message: error.user_message(),
                });
            }
            Err(_) => {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: crate::i18n::tr("Extraction stopped unexpectedly"),
                    password_failure: None,
                });
            }
        }
    });

    LoadHandle::new(move || {
        cancelled.store(true, Ordering::Relaxed);
    })
}

fn extract_named_folder(
    destination: &Path,
    archive_name: &str,
    cancelled: &AtomicBool,
    decode: impl FnOnce(&Path) -> Result<Option<String>, ArchiveError>,
) -> Result<Option<String>, ArchiveError> {
    let format = ArchiveFormat::from_extension(archive_name).ok_or_else(|| {
        ArchiveError::Failed(
            rust_i18n::t!("Unsupported archive format: %{name}", name = archive_name).into_owned(),
        )
    })?;
    let extension = if archive_name.to_ascii_lowercase().ends_with(".tgz") {
        4
    } else {
        format.extension().len() + 1
    };
    let stem = &archive_name[..archive_name.len() - extension];
    let stem = if stem.is_empty() { archive_name } else { stem };
    validate_basename(stem).map_err(|message| ArchiveError::Failed(crate::i18n::tr(message)))?;
    for suffix in 0_u64.. {
        check_cancelled(cancelled)?;
        let name = if suffix == 0 {
            stem.to_owned()
        } else {
            format!("{stem} ({suffix})")
        };
        let folder = destination.join(&name);
        match std::fs::create_dir(&folder) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(ArchiveError::Failed(crate::services::io_error_message(
                    &error,
                )));
            }
            Ok(()) => {}
        }
        let result = decode(&folder);
        if result.is_err() {
            let _ = std::fs::remove_dir(&folder);
        }
        return result.map(|_| Some(name));
    }
    unreachable!("unbounded folder suffixes")
}

fn extract_zip(
    archive_path: &Path,
    dest_dir: &Path,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    total: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<Option<String>, ArchiveError> {
    let file = File::open(archive_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_decode_error)?;
    total.store(archive.len(), Ordering::Relaxed);

    let mut resolver = WindowsExtractResolver::new();
    let mut first_name: Option<String> = None;

    let pw_bytes = password.map(str::as_bytes);
    for i in 0..archive.len() {
        check_cancelled(cancelled)?;

        let options = zip::read::ZipReadOptions::new().password(pw_bytes);
        let mut entry = match archive.by_index_with_options(i, options) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::InvalidPassword) => {
                return Err(ArchiveError::IncorrectPassword(
                    MAYBE_BAD_PASSWORD.to_string(),
                ));
            }
            Err(e) => return Err(zip_decode_error(e)),
        };

        let raw_name = entry.name().to_string();
        let clean_path = sanitize_archive_member_path(&raw_name).map_err(ArchiveError::Failed)?;

        let resolved_rel = resolver
            .resolve(dest_dir, &clean_path)
            .map_err(ArchiveError::Failed)?;

        if first_name.is_none() {
            if let Some(top) = resolved_rel.components().next() {
                first_name = Some(top.as_os_str().to_string_lossy().into_owned());
            }
        }

        let target_path = dest_dir.join(&resolved_rel);

        if entry.is_dir() {
            std::fs::create_dir_all(&target_path)
                .map_err(|e| ArchiveError::Failed(e.to_string()))?;
        } else {
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| ArchiveError::Failed(e.to_string()))?;
            }

            let mut outfile =
                File::create(&target_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;

            let mut buf = vec![0u8; COPY_BUF_SIZE];
            loop {
                check_cancelled(cancelled)?;
                let n = entry.read(&mut buf).map_err(|e| {
                    if e.kind() == io::ErrorKind::InvalidData {
                        ArchiveError::IncorrectPassword(MAYBE_BAD_PASSWORD.to_string())
                    } else {
                        ArchiveError::Failed(e.to_string())
                    }
                })?;
                if n == 0 {
                    break;
                }
                outfile
                    .write_all(&buf[..n])
                    .map_err(|e| ArchiveError::Failed(e.to_string()))?;
            }
        }

        progress.fetch_add(1, Ordering::Relaxed);
    }

    Ok(first_name)
}

fn zip_decode_error(e: zip::result::ZipError) -> ArchiveError {
    match e {
        zip::result::ZipError::UnsupportedArchive(zip::result::ZipError::PASSWORD_REQUIRED) => {
            ArchiveError::PasswordRequired(PASSWORD_REQUIRED.to_string())
        }
        zip::result::ZipError::InvalidPassword => {
            ArchiveError::IncorrectPassword(MAYBE_BAD_PASSWORD.to_string())
        }
        _ => ArchiveError::Failed(e.to_string()),
    }
}

fn extract_tar(
    archive_path: &Path,
    dest_dir: &Path,
    gzip: bool,
    progress: &Arc<AtomicUsize>,
    total: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<Option<String>, ArchiveError> {
    let file = File::open(archive_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;
    let reader: Box<dyn Read> = if gzip {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };

    let mut archive = tar::Archive::new(reader);
    let mut resolver = WindowsExtractResolver::new();
    let mut first_name: Option<String> = None;

    let entries = archive
        .entries()
        .map_err(|e| ArchiveError::Failed(e.to_string()))?;
    for entry_res in entries {
        check_cancelled(cancelled)?;
        let mut entry = entry_res.map_err(|e| ArchiveError::Failed(e.to_string()))?;

        let entry_path = entry
            .path()
            .map_err(|e| ArchiveError::Failed(e.to_string()))?;
        let path_str = entry_path.to_string_lossy().to_string();
        let clean_path = sanitize_archive_member_path(&path_str).map_err(ArchiveError::Failed)?;

        let resolved_rel = resolver
            .resolve(dest_dir, &clean_path)
            .map_err(ArchiveError::Failed)?;

        if first_name.is_none() {
            if let Some(top) = resolved_rel.components().next() {
                first_name = Some(top.as_os_str().to_string_lossy().into_owned());
            }
        }

        let target_path = dest_dir.join(&resolved_rel);
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target_path)
                .map_err(|e| ArchiveError::Failed(e.to_string()))?;
        } else {
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| ArchiveError::Failed(e.to_string()))?;
            }

            let mut outfile =
                File::create(&target_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;

            let mut buf = vec![0u8; COPY_BUF_SIZE];
            loop {
                check_cancelled(cancelled)?;
                let n = entry
                    .read(&mut buf)
                    .map_err(|e| ArchiveError::Failed(e.to_string()))?;
                if n == 0 {
                    break;
                }
                outfile
                    .write_all(&buf[..n])
                    .map_err(|e| ArchiveError::Failed(e.to_string()))?;
            }
        }

        progress.fetch_add(1, Ordering::Relaxed);
        total.fetch_max(progress.load(Ordering::Relaxed), Ordering::Relaxed);
    }

    Ok(first_name)
}

fn extract_sevenz(
    archive_path: &Path,
    dest_dir: &Path,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    total: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<Option<String>, ArchiveError> {
    let pw = password
        .map(sevenz_rust2::Password::from)
        .unwrap_or_default();
    let file = File::open(archive_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;

    let mut archive = sevenz_rust2::ArchiveReader::new(file, pw)
        .map_err(|e| ArchiveError::Failed(e.to_string()))?;

    total.store(archive.archive().files.len(), Ordering::Relaxed);

    let mut resolver = WindowsExtractResolver::new();
    let mut first_name: Option<String> = None;

    archive
        .for_each_entries(|entry, reader| {
            if cancelled.load(Ordering::Relaxed) {
                return Err(sevenz_rust2::Error::Other(std::borrow::Cow::Borrowed(
                    "Cancelled",
                )));
            }

            let name = entry.name().to_string();
            let clean_path = match sanitize_archive_member_path(&name) {
                Ok(p) => p,
                Err(e) => return Err(sevenz_rust2::Error::Other(std::borrow::Cow::Owned(e))),
            };

            let resolved_rel = match resolver.resolve(dest_dir, &clean_path) {
                Ok(p) => p,
                Err(e) => return Err(sevenz_rust2::Error::Other(std::borrow::Cow::Owned(e))),
            };

            if first_name.is_none() {
                if let Some(top) = resolved_rel.components().next() {
                    first_name = Some(top.as_os_str().to_string_lossy().into_owned());
                }
            }

            let target_path = dest_dir.join(&resolved_rel);

            if entry.is_directory() {
                if let Err(e) = std::fs::create_dir_all(&target_path) {
                    return Err(sevenz_rust2::Error::Io(
                        e,
                        std::borrow::Cow::Borrowed("mkdir"),
                    ));
                }
            } else {
                if let Some(parent) = target_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return Err(sevenz_rust2::Error::Io(
                            e,
                            std::borrow::Cow::Borrowed("mkdir"),
                        ));
                    }
                }

                let mut outfile = match File::create(&target_path) {
                    Ok(f) => f,
                    Err(e) => {
                        return Err(sevenz_rust2::Error::Io(
                            e,
                            std::borrow::Cow::Borrowed("create"),
                        ));
                    }
                };

                let mut buf = vec![0u8; COPY_BUF_SIZE];
                loop {
                    if cancelled.load(Ordering::Relaxed) {
                        return Err(sevenz_rust2::Error::Other(std::borrow::Cow::Borrowed(
                            "Cancelled",
                        )));
                    }
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if let Err(e) = outfile.write_all(&buf[..n]) {
                                return Err(sevenz_rust2::Error::Io(
                                    e,
                                    std::borrow::Cow::Borrowed("write"),
                                ));
                            }
                        }
                        Err(e) => {
                            return Err(sevenz_rust2::Error::Io(
                                e,
                                std::borrow::Cow::Borrowed("read"),
                            ));
                        }
                    }
                }
            }

            progress.fetch_add(1, Ordering::Relaxed);
            Ok(true)
        })
        .map_err(|e| {
            if cancelled.load(Ordering::Relaxed) {
                ArchiveError::Cancelled
            } else {
                ArchiveError::Failed(e.to_string())
            }
        })?;

    Ok(first_name)
}

// ---------------------------------------------------------------------------
// Compression
// ---------------------------------------------------------------------------

pub(crate) fn compress(request: CompressRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
    let cancelled = Arc::new(AtomicBool::new(false));
    let task_cancelled = cancelled.clone();
    let work_cancelled = cancelled.clone();
    let destination = request.destination.clone();
    let _source_locations = request
        .entries
        .iter()
        .map(|entry| entry.location.clone())
        .collect::<Vec<_>>();

    let _task = glib::MainContext::default().spawn_local(async move {
        let Some(dest_dir) = request.destination.native_path().map(Path::to_path_buf) else {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: crate::i18n::tr("Archive destination must be a local path"),
                password_failure: None,
            });
            return;
        };

        if let Err(message) = validate_basename(&request.archive_name) {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: crate::i18n::tr(message),
                password_failure: None,
            });
            return;
        }

        let entries: Vec<PathBuf> = request
            .entries
            .iter()
            .filter_map(|e| e.location.native_path().map(Path::to_path_buf))
            .collect();

        if entries.is_empty() {
            emit(OperationEvent::Failed {
                request_id: request.id,
                message: crate::i18n::tr("Nothing to compress"),
                password_failure: None,
            });
            return;
        }

        let total = Arc::new(AtomicUsize::new(0));
        let progress = Arc::new(AtomicUsize::new(0));
        emit(OperationEvent::ArchiveStarted {
            request_id: request.id,
            total: 0,
        });

        let timer_id =
            archive_progress_timer(request.id, &progress, &total, &task_cancelled, &emit);

        let format = request.format;
        let password = request.password.clone();
        let archive_base_name = request.archive_name.clone();
        let conflict = request.conflict;
        let work_progress = progress.clone();
        let work_total = total.clone();
        let worker_dest = dest_dir.clone();

        let result = gio::spawn_blocking(move || {
            let sources = collect_sources(&entries, &work_cancelled)?;
            work_total.store(sources.len(), Ordering::Relaxed);

            // Staging file in destination directory
            let staging_name = format!(
                ".strata-compress-{}-{}.tmp",
                std::process::id(),
                rand_suffix()
            );
            let staging_path = worker_dest.join(&staging_name);

            let compress_res = match format {
                ArchiveFormat::Zip => compress_zip(
                    &staging_path,
                    &sources,
                    password.as_deref(),
                    &work_progress,
                    &work_cancelled,
                ),
                ArchiveFormat::Tar => compress_tar(
                    &staging_path,
                    &sources,
                    false,
                    &work_progress,
                    &work_cancelled,
                ),
                ArchiveFormat::TarGz => compress_tar(
                    &staging_path,
                    &sources,
                    true,
                    &work_progress,
                    &work_cancelled,
                ),
                ArchiveFormat::SevenZ => compress_sevenz(
                    &staging_path,
                    &sources,
                    password.as_deref(),
                    &work_progress,
                    &work_cancelled,
                ),
                ArchiveFormat::Rar => Err(ArchiveError::Failed(crate::i18n::tr(
                    "RAR compression is not supported",
                ))),
            };

            if let Err(e) = compress_res {
                let _ = std::fs::remove_file(&staging_path);
                return Err(e);
            }

            // Conflict resolution and atomic rename
            let extension = format.extension();
            let base_file_name = format!("{archive_base_name}.{extension}");
            let target_path = worker_dest.join(&base_file_name);

            let final_path = match conflict {
                TransferConflict::FailIfExists => {
                    if target_path.exists() {
                        let _ = std::fs::remove_file(&staging_path);
                        return Err(ArchiveError::Failed(format!(
                            "Destination file already exists: {base_file_name}"
                        )));
                    }
                    target_path
                }
                TransferConflict::ReplaceExisting => {
                    if target_path.exists() {
                        let _ = strata_win_shell::recycle(&[target_path.clone()]);
                    }
                    target_path
                }
                TransferConflict::KeepBoth => {
                    if target_path.exists() {
                        let available =
                            find_available_name(&worker_dest, OsStr::new(&base_file_name))
                                .map_err(ArchiveError::Failed)?;
                        worker_dest.join(available)
                    } else {
                        target_path
                    }
                }
                TransferConflict::Merge => {
                    // For archive output, merge treats as replace
                    if target_path.exists() {
                        let _ = strata_win_shell::recycle(&[target_path.clone()]);
                    }
                    target_path
                }
            };

            if let Err(e) = std::fs::rename(&staging_path, &final_path) {
                let _ = std::fs::remove_file(&staging_path);
                return Err(ArchiveError::Failed(format!(
                    "Failed to publish archive: {e}"
                )));
            }

            let final_archive_name = final_path
                .file_name()
                .unwrap_or(OsStr::new(&base_file_name))
                .to_string_lossy()
                .into_owned();

            Ok(final_archive_name)
        })
        .await;

        timer_id.remove();

        match result {
            Ok(Ok(archive_name)) => {
                emit(OperationEvent::Compressed {
                    request_id: request.id,
                    original: None,
                    archive: Location::local(dest_dir.join(&archive_name)),
                    archive_name,
                });
            }
            Ok(Err(ArchiveError::Cancelled)) => {
                emit(OperationEvent::Cancelled {
                    request_id: request.id,
                    result: CancelledOperation {
                        completed: Vec::new(),
                        failed: Vec::new(),
                        not_attempted: Vec::new(),
                        affected_locations: std::collections::HashSet::from([destination]),
                    },
                });
            }
            Ok(Err(error)) => {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: error.user_message(),
                    password_failure: None,
                });
            }
            Err(_) => {
                emit(OperationEvent::Failed {
                    request_id: request.id,
                    message: crate::i18n::tr("Compression stopped unexpectedly"),
                    password_failure: None,
                });
            }
        }
    });

    LoadHandle::new(move || {
        cancelled.store(true, Ordering::Relaxed);
    })
}

fn rand_suffix() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(12345)
}

struct SourceItem {
    disk_path: PathBuf,
    archive_path: String,
    is_dir: bool,
}

fn collect_sources(
    entries: &[PathBuf],
    cancelled: &AtomicBool,
) -> Result<Vec<SourceItem>, ArchiveError> {
    let mut items = Vec::new();
    for entry in entries {
        check_cancelled(cancelled)?;
        let name = entry
            .file_name()
            .ok_or_else(|| ArchiveError::Failed("Root path has no filename".to_string()))?
            .to_string_lossy()
            .into_owned();

        let meta =
            std::fs::symlink_metadata(entry).map_err(|e| ArchiveError::Failed(e.to_string()))?;

        if meta.is_dir() {
            items.push(SourceItem {
                disk_path: entry.clone(),
                archive_path: format!("{name}/"),
                is_dir: true,
            });
            walk_dir_recursive(entry, &name, &mut items, cancelled)?;
        } else if meta.is_file() {
            items.push(SourceItem {
                disk_path: entry.clone(),
                archive_path: name,
                is_dir: false,
            });
        }
    }
    Ok(items)
}

fn walk_dir_recursive(
    dir: &Path,
    parent_rel: &str,
    items: &mut Vec<SourceItem>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    check_cancelled(cancelled)?;
    let read_dir = std::fs::read_dir(dir).map_err(|e| ArchiveError::Failed(e.to_string()))?;
    for child in read_dir {
        check_cancelled(cancelled)?;
        let child = child.map_err(|e| ArchiveError::Failed(e.to_string()))?;
        let child_path = child.path();
        let child_name = child.file_name().to_string_lossy().into_owned();
        let rel_path = format!("{parent_rel}/{child_name}");

        let meta = std::fs::symlink_metadata(&child_path)
            .map_err(|e| ArchiveError::Failed(e.to_string()))?;

        // Do not traverse reparse points / symlinks pointing elsewhere
        if meta.is_symlink() {
            continue;
        }

        if meta.is_dir() {
            items.push(SourceItem {
                disk_path: child_path.clone(),
                archive_path: format!("{rel_path}/"),
                is_dir: true,
            });
            walk_dir_recursive(&child_path, &rel_path, items, cancelled)?;
        } else if meta.is_file() {
            items.push(SourceItem {
                disk_path: child_path,
                archive_path: rel_path,
                is_dir: false,
            });
        }
    }
    Ok(())
}

fn compress_zip(
    staging_path: &Path,
    sources: &[SourceItem],
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    let file = File::create(staging_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;
    let mut writer = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let options = if let Some(pw) = password {
        options.with_aes_encryption(zip::AesMode::Aes256, pw)
    } else {
        options
    };

    for item in sources {
        check_cancelled(cancelled)?;
        if item.is_dir {
            writer
                .add_directory(&item.archive_path, options)
                .map_err(|e| ArchiveError::Failed(e.to_string()))?;
        } else {
            writer
                .start_file(&item.archive_path, options)
                .map_err(|e| ArchiveError::Failed(e.to_string()))?;

            let mut reader =
                File::open(&item.disk_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;
            let mut buf = vec![0u8; COPY_BUF_SIZE];
            loop {
                check_cancelled(cancelled)?;
                let n = reader
                    .read(&mut buf)
                    .map_err(|e| ArchiveError::Failed(e.to_string()))?;
                if n == 0 {
                    break;
                }
                writer
                    .write_all(&buf[..n])
                    .map_err(|e| ArchiveError::Failed(e.to_string()))?;
            }
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }

    writer
        .finish()
        .map_err(|e| ArchiveError::Failed(e.to_string()))?;
    Ok(())
}

fn compress_tar(
    staging_path: &Path,
    sources: &[SourceItem],
    gzip: bool,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    let file = File::create(staging_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;
    let writer: Box<dyn Write> = if gzip {
        Box::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::default(),
        ))
    } else {
        Box::new(file)
    };

    let mut tar = tar::Builder::new(writer);

    for item in sources {
        check_cancelled(cancelled)?;
        if item.is_dir {
            tar.append_dir(&item.archive_path, &item.disk_path)
                .map_err(|e| ArchiveError::Failed(e.to_string()))?;
        } else {
            let mut file =
                File::open(&item.disk_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;
            tar.append_file(&item.archive_path, &mut file)
                .map_err(|e| ArchiveError::Failed(e.to_string()))?;
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }

    tar.finish()
        .map_err(|e| ArchiveError::Failed(e.to_string()))?;
    Ok(())
}

fn compress_sevenz(
    staging_path: &Path,
    sources: &[SourceItem],
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    cancelled: &AtomicBool,
) -> Result<(), ArchiveError> {
    let file = File::create(staging_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;
    let mut writer =
        sevenz_rust2::ArchiveWriter::new(file).map_err(|e| ArchiveError::Failed(e.to_string()))?;

    if let Some(pw) = password {
        writer.set_content_methods(vec![
            sevenz_rust2::encoder_options::AesEncoderOptions::new(pw.into()).into(),
            sevenz_rust2::EncoderConfiguration::new(sevenz_rust2::EncoderMethod::LZMA2),
        ]);
    } else {
        writer.set_content_methods(vec![sevenz_rust2::EncoderConfiguration::new(
            sevenz_rust2::EncoderMethod::LZMA2,
        )]);
    }

    for item in sources {
        check_cancelled(cancelled)?;
        if item.is_dir {
            let entry = sevenz_rust2::ArchiveEntry::new_directory(&item.archive_path);
            writer
                .push_archive_entry(entry, None::<File>)
                .map_err(|e| ArchiveError::Failed(e.to_string()))?;
        } else {
            let entry = sevenz_rust2::ArchiveEntry::new_file(&item.archive_path);
            let file =
                File::open(&item.disk_path).map_err(|e| ArchiveError::Failed(e.to_string()))?;
            writer
                .push_archive_entry(entry, Some(file))
                .map_err(|e| ArchiveError::Failed(e.to_string()))?;
        }
        progress.fetch_add(1, Ordering::Relaxed);
    }

    writer
        .finish()
        .map_err(|e| ArchiveError::Failed(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_sanitize_archive_member_path_traversal_rejection() {
        assert!(sanitize_archive_member_path("../evil.txt").is_err());
        assert!(sanitize_archive_member_path("folder/../../evil.txt").is_err());
        assert!(sanitize_archive_member_path("..\\evil.txt").is_err());
        assert!(sanitize_archive_member_path("/absolute/path").is_err());
        assert!(sanitize_archive_member_path("\\\\server\\share").is_err());
        assert!(sanitize_archive_member_path("C:\\windows\\system32").is_err());
        assert!(sanitize_archive_member_path("D:file.txt").is_err());
        assert!(sanitize_archive_member_path("file.txt:stream").is_err());

        // Normal paths
        let clean = sanitize_archive_member_path("folder/sub/file.txt").unwrap();
        assert_eq!(clean, PathBuf::from("folder/sub/file.txt"));

        let clean_win = sanitize_archive_member_path("folder\\sub\\file.txt").unwrap();
        assert_eq!(clean_win, PathBuf::from("folder/sub/file.txt"));
    }

    #[test]
    fn test_sanitize_archive_member_path_reserved_device_names() {
        assert!(sanitize_archive_member_path("CON").is_err());
        assert!(sanitize_archive_member_path("con.txt").is_err());
        assert!(sanitize_archive_member_path("NUL").is_err());
        assert!(sanitize_archive_member_path("folder/aux.png").is_err());
        assert!(sanitize_archive_member_path("com1").is_err());
        assert!(sanitize_archive_member_path("lpt1.txt").is_err());

        // Allowed
        assert!(sanitize_archive_member_path("connect.txt").is_ok());
        assert!(sanitize_archive_member_path("normal_file.txt").is_ok());
    }

    #[test]
    fn test_windows_zip_compression_and_extraction_roundtrip() {
        let temp = tempfile::tempdir().expect("tempdir");
        let src_dir = temp.path().join("source");
        fs::create_dir_all(&src_dir).expect("src_dir");

        let file1 = src_dir.join("test1.txt");
        let file2 = src_dir.join("test2.txt");
        fs::write(&file1, b"Hello World 1").expect("write1");
        fs::write(&file2, b"Hello World 2").expect("write2");

        let zip_path = temp.path().join("out.zip");
        let cancelled = AtomicBool::new(false);
        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));

        let sources = collect_sources(&[src_dir.clone()], &cancelled).expect("collect");
        assert_eq!(sources.len(), 3); // 1 dir + 2 files

        compress_zip(&zip_path, &sources, None, &progress, &cancelled).expect("compress");
        assert!(zip_path.exists());

        let extract_dir = temp.path().join("extracted");
        fs::create_dir_all(&extract_dir).expect("extract_dir");

        let first = extract_zip(&zip_path, &extract_dir, None, &progress, &total, &cancelled)
            .expect("extract");

        assert_eq!(first, Some("source".to_string()));
        assert_eq!(
            fs::read(extract_dir.join("source/test1.txt")).expect("read1"),
            b"Hello World 1"
        );
        assert_eq!(
            fs::read(extract_dir.join("source/test2.txt")).expect("read2"),
            b"Hello World 2"
        );
    }

    #[test]
    fn test_windows_targz_compression_and_extraction_roundtrip() {
        let temp = tempfile::tempdir().expect("tempdir");
        let src_dir = temp.path().join("tarsrc");
        fs::create_dir_all(&src_dir).expect("src_dir");

        let file1 = src_dir.join("doc.md");
        fs::write(&file1, b"# Markdown doc").expect("write doc");

        let targz_path = temp.path().join("out.tar.gz");
        let cancelled = AtomicBool::new(false);
        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));

        let sources = collect_sources(&[src_dir.clone()], &cancelled).expect("collect");
        compress_tar(&targz_path, &sources, true, &progress, &cancelled).expect("compress");
        assert!(targz_path.exists());

        let extract_dir = temp.path().join("extracted_tar");
        fs::create_dir_all(&extract_dir).expect("extract_dir");

        let first = extract_tar(
            &targz_path,
            &extract_dir,
            true,
            &progress,
            &total,
            &cancelled,
        )
        .expect("extract");

        assert_eq!(first, Some("tarsrc".to_string()));
        assert_eq!(
            fs::read(extract_dir.join("tarsrc/doc.md")).expect("read doc"),
            b"# Markdown doc"
        );
    }

    #[test]
    fn test_windows_extract_keep_both_auto_rename() {
        let temp = tempfile::tempdir().expect("tempdir");
        let extract_dir = temp.path().join("dest");
        fs::create_dir_all(&extract_dir).expect("dest");

        // Pre-create "item.txt"
        fs::write(extract_dir.join("item.txt"), b"Original").expect("write original");

        let mut resolver = WindowsExtractResolver::new();
        let target1 = resolver
            .resolve(&extract_dir, Path::new("item.txt"))
            .expect("resolve 1");
        assert_eq!(target1, PathBuf::from("item (1).txt"));

        // Pre-create "item (1).txt"
        fs::write(extract_dir.join("item (1).txt"), b"Copy 1").expect("write copy 1");

        let mut resolver2 = WindowsExtractResolver::new();
        let target2 = resolver2
            .resolve(&extract_dir, Path::new("item.txt"))
            .expect("resolve 2");
        assert_eq!(target2, PathBuf::from("item (2).txt"));
    }
}
