// SPDX-License-Identifier: MIT

use gtk::gio;
use std::io;
#[cfg(any(windows, test))]
use std::path::Path;

pub(super) fn clear(done: impl FnOnce(io::Result<()>) + 'static) {
    crate::adapters::clear_preview_cache();
    super::thumbnail::clear_memory_cache();
    gtk::glib::MainContext::default().spawn_local(async move {
        let usage = crate::services::disk_usage::clear_cache().await;
        let result = gio::spawn_blocking(super::thumbnail_cache::clear_owned)
            .await
            .unwrap_or_else(|_| Err(io::Error::from(io::ErrorKind::Interrupted)));
        if let Err(error) = &result {
            tracing::warn!(%error, "unable to clear preview cache");
        }
        done(result.and(usage));
    });
}

#[cfg(any(windows, test))]
pub(super) fn clear_thumbnail_files(directory: &Path) -> io::Result<()> {
    if !directory.is_absolute() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    for ancestor in directory.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                #[cfg(windows)]
                let link = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let link = metadata.file_type().is_symlink();
                if link {
                    return Err(io::Error::from(io::ErrorKind::InvalidInput));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(key) = name.strip_suffix(".thumb") else {
            continue;
        };
        if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        if entry.file_type()?.is_file() {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
