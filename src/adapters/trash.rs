// SPDX-License-Identifier: MIT

//! Bounded GIO Trash deletion, independent of browser widgets.

use gio::prelude::*;

pub(crate) struct EmptyTrashOutcome {
    pub(crate) deleted: usize,
    pub(crate) failed: usize,
    /// Capped at 8 messages regardless of `failed`.
    pub(crate) errors: Vec<String>,
}

/// Deletes one batch at a time, independently of any prior measurement's budget or truncation.
pub(crate) async fn empty_trash(
    root: &gio::File,
    mut on_progress: impl FnMut(usize),
) -> Result<EmptyTrashOutcome, glib::Error> {
    #[cfg(windows)]
    if root.uri().as_str() == "trash:///" {
        let listing = super::recycle_bin::listing().await?;
        if listing.truncated {
            return Err(super::recycle_bin::error(crate::i18n::tr(
                "Unable to read Trash",
            )));
        }
        let mut outcome = EmptyTrashOutcome {
            deleted: 0,
            failed: 0,
            errors: Vec::new(),
        };
        for entry in listing.entries {
            let result = gio::spawn_blocking(move || {
                strata_win_shell::delete_recycled(&entry.item.recycled)
            })
            .await;
            match result {
                Ok(Ok(())) => outcome.deleted += 1,
                result => {
                    outcome.failed += 1;
                    if outcome.errors.len() < 8 {
                        outcome.errors.push(match result {
                            Ok(Err(error)) => super::recycle_bin::operation_error_message(&error),
                            _ => crate::i18n::tr("Unable to read Trash"),
                        });
                    }
                }
            }
            on_progress(outcome.deleted + outcome.failed);
        }
        return Ok(outcome);
    }
    let enumerator = root
        .enumerate_children_future(
            "standard::name,standard::display-name",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?;
    let mut outcome = EmptyTrashOutcome {
        deleted: 0,
        failed: 0,
        errors: Vec::new(),
    };
    loop {
        let children = enumerator
            .next_files_future(64, glib::Priority::DEFAULT)
            .await?;
        if children.is_empty() {
            break;
        }
        for info in children {
            let Some(name) = info
                .has_attribute(gio::FILE_ATTRIBUTE_STANDARD_NAME)
                .then(|| info.name())
            else {
                outcome.failed += 1;
                continue;
            };
            let file = root.child(&name);
            match file.delete_future(glib::Priority::DEFAULT).await {
                Ok(_) => outcome.deleted += 1,
                Err(error) => {
                    outcome.failed += 1;
                    if outcome.errors.len() < 8 {
                        let display_name =
                            if info.has_attribute(gio::FILE_ATTRIBUTE_STANDARD_DISPLAY_NAME) {
                                info.display_name().to_string()
                            } else {
                                name.to_string_lossy().into_owned()
                            };
                        outcome.errors.push(format!("{display_name}: {error}"));
                    }
                }
            }
        }
        on_progress(outcome.deleted + outcome.failed);
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests;
