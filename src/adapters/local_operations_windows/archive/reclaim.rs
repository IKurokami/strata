// SPDX-License-Identifier: MIT

use super::{ArchiveError, check_cancelled};
use hatchzip_core::{
    archive::{PasswordNeeded, WrongPassword, storage::Reads},
    engine::{self, Outcome, Progress, Run},
    journal::Journal,
    session::{self, State},
};
use std::{
    os::windows::fs::MetadataExt,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

const KEEP_FREE: u64 = 256 * 1024 * 1024;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

fn unsafe_paths() -> ArchiveError {
    ArchiveError::Failed(crate::i18n::tr(
        "Space-saving extraction cannot safely use these paths. Use normal extraction instead.",
    ))
}

fn check_output_paths(job: &engine::Job, output: &Path) -> Result<(), ArchiveError> {
    for item in job.units.iter().flat_map(|unit| &unit.items) {
        if !matches!(
            item.kind,
            hatchzip_core::archive::Kind::File | hatchzip_core::archive::Kind::Dir
        ) {
            return Err(unsafe_paths());
        }
        let path = output.join(&item.path);
        for ancestor in path.ancestors() {
            if std::fs::symlink_metadata(ancestor).is_ok_and(|metadata| {
                metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            }) {
                return Err(unsafe_paths());
            }
        }
    }
    Ok(())
}

fn failure(error: impl std::fmt::Display) -> ArchiveError {
    ArchiveError::Failed(
        rust_i18n::t!(
            "Space-saving extraction failed: %{error}",
            error = error.to_string()
        )
        .into_owned(),
    )
}

pub(super) fn extract(
    selected: &Path,
    password: Option<&str>,
    progress: &Arc<AtomicUsize>,
    total: &AtomicUsize,
    cancelled: &AtomicBool,
) -> Result<Option<String>, ArchiveError> {
    check_cancelled(cancelled)?;
    // The journal remains selectable even if a crash happened after deleting
    // the final archive volume but before removing the recovery files.
    let archive = if selected
        .extension()
        .is_some_and(|extension| extension == "hatchzip")
    {
        selected.with_extension("")
    } else {
        selected.to_path_buf()
    };
    if std::fs::symlink_metadata(&archive)
        .is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
    {
        return Err(unsafe_paths());
    }
    let (primary, paths) = session::locate(&archive).map_err(failure)?;
    if paths.exists() {
        let (journal, _) = Journal::load(&paths).map_err(failure)?;
        if journal.parts.len() != 1
            || journal.parts[0].0 != primary
            || journal.output != session::default_output(&primary)
            || !matches!(journal.format.as_str(), "zip" | "7z" | "tar")
            || !journal.links.is_empty()
            || journal.restoring.is_some()
        {
            return Err(ArchiveError::Failed(crate::i18n::tr(
                "Recovery files do not match this archive.",
            )));
        }
    }
    let (state, _) = session::open_encrypted(
        &primary,
        &paths,
        None,
        true,
        password.map(str::to_owned),
        &Arc::new(Reads::default()),
        |_| None,
    )
    .map_err(|error| {
        if error.is::<PasswordNeeded>() {
            ArchiveError::PasswordRequired(super::PASSWORD_REQUIRED.into())
        } else if error.is::<WrongPassword>() {
            ArchiveError::IncorrectPassword(super::MAYBE_BAD_PASSWORD.into())
        } else {
            failure(error)
        }
    })?;
    check_cancelled(cancelled)?;
    let (job, journal) = match state {
        State::Orphan(journal) => {
            engine::finish_orphaned(&journal, &paths).map_err(failure)?;
            return Ok(journal
                .output
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()));
        }
        State::Fresh(job, output) => {
            if job.storage.parts().len() != 1 || !matches!(job.format_name, "zip" | "7z" | "tar") {
                return Err(ArchiveError::Failed(crate::i18n::tr(
                    "Space-saving extraction supports single-volume ZIP, 7z and TAR archives.",
                )));
            }
            check_output_paths(&job, &output)?;
            let plan = session::plan(&job, &output, false, KEEP_FREE, false).map_err(failure)?;
            if !plan.fits {
                return Err(ArchiveError::Failed(crate::i18n::tr(
                    "Not enough free space for space-saving extraction. The archive was not changed.",
                )));
            }
            check_cancelled(cancelled)?;
            let journal = session::begin(&job, &paths, output, false).map_err(failure)?;
            (job, journal)
        }
        State::Resume(job, journal) => {
            check_output_paths(&job, &journal.output)?;
            let plan = session::plan(&job, &journal.output, true, KEEP_FREE, journal.keep)
                .map_err(failure)?;
            if !plan.fits {
                return Err(ArchiveError::Failed(crate::i18n::tr(
                    "Not enough free space to resume extraction. Keep the archive, output folder and recovery files together.",
                )));
            }
            session::clear_uncommitted(&job, &journal.output).map_err(failure)?;
            (job, journal)
        }
    };
    let name = journal
        .output
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    // Progress counts entries, matching the existing archive operation UI.
    let units: Vec<_> = job
        .units
        .iter()
        .map(|unit| (unit.start, unit.items.len()))
        .collect();
    total.store(job.item_count(), Ordering::Relaxed);
    progress.store(0, Ordering::Relaxed);
    let progress = progress.clone();
    let result = Run {
        job,
        journal,
        paths,
        stop: cancelled,
        keep_free: KEEP_FREE,
        progress: Progress {
            on_bytes: Box::new(|_| {}),
            on_commit: Box::new(move |length| {
                progress.store(
                    units
                        .iter()
                        .filter(|(start, _)| *start >= length)
                        .map(|(_, count)| count)
                        .sum(),
                    Ordering::Relaxed,
                );
            }),
        },
    }
    .execute()
    .map_err(failure)?;
    match result {
        Outcome::Finished => Ok(name),
        Outcome::Paused => Err(ArchiveError::Cancelled),
    }
}

#[cfg(test)]
#[path = "reclaim/tests.rs"]
mod tests;
