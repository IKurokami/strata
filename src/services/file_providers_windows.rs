// SPDX-License-Identifier: MIT
//! Win32 opt-in external file providers. Providers are discovered only from
//! the per-user configuration directory and invoked through a bounded Job
//! Object transport; no shell expansion or PATH lookup is permitted.
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use std::os::windows::fs::MetadataExt;

#[path = "file_providers/protocol.rs"]
mod protocol;
#[path = "file_providers/transport_windows.rs"]
mod transport;

pub(crate) use protocol::{
    Decoration, Feedback, MenuAction, OutcomeStatus, Reply, Request, query_batch, selection_fits,
};
pub(crate) use transport::{Client, Update, start};
pub(crate) const PATH_LIMIT: usize = 200;

#[derive(Clone, Deserialize)]
pub(crate) struct Manifest {
    pub version: u32,
    pub id: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub command: Vec<String>,
    #[serde(default)]
    pub icons: BTreeMap<String, String>,
}

#[derive(Clone)]
pub(crate) struct Registration {
    pub manifest: Manifest,
    pub icons: BTreeMap<String, Vec<u8>>,
}

/// The wire protocol uses the same bounded identifier rules on every platform.
fn slug(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-')
}

const MAX_MANIFEST_BYTES: u64 = 16 * 1024;
const MAX_ICON_BYTES: u64 = 64 * 1024;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

fn trusted(path: &Path, directory: bool) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
            && metadata.file_type().is_dir() == directory
            && (directory || metadata.is_file())
            && strata_win_shell::path_owned_by_current_user(path)
    })
}

fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= limit as usize).then_some(bytes)
}

fn load(directory: &Path) -> Option<Registration> {
    if !trusted(directory, true) || !trusted(&directory.join("provider.json"), false) {
        return None;
    }
    let manifest: Manifest = serde_json::from_slice(&read_limited(
        &directory.join("provider.json"),
        MAX_MANIFEST_BYTES,
    )?)
    .ok()?;
    if manifest.version != 1
        || !slug(&manifest.id)
        || directory.file_name()?.to_str()? != manifest.id
        || manifest.name.as_ref().is_some_and(|name| {
            name.trim().is_empty() || name.len() > 64 || name.chars().any(char::is_control)
        })
        || manifest.command.is_empty()
        || manifest.command.len() > 16
        || manifest.icons.len() > 16
        || manifest
            .icon
            .as_ref()
            .is_some_and(|icon| !manifest.icons.contains_key(icon))
        || manifest
            .command
            .iter()
            .any(|argument| argument.len() > 4096 || argument.contains('\0'))
    {
        return None;
    }

    let executable = Path::new(&manifest.command[0]);
    if !executable.is_absolute() || !trusted(executable, false) {
        return None;
    }

    let mut icons = BTreeMap::new();
    for (id, filename) in &manifest.icons {
        if !slug(id) || Path::new(filename).components().count() != 1 || !filename.ends_with(".png")
        {
            return None;
        }
        let path = directory.join(filename);
        let bytes = read_limited(&path, MAX_ICON_BYTES)?;
        if !trusted(&path, false) || bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
            return None;
        }
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        if width == 0 || height == 0 || width > 256 || height > 256 {
            return None;
        }
        icons.insert(id.clone(), bytes);
    }
    Some(Registration { manifest, icons })
}

pub(crate) fn discover(root: &Path) -> Vec<Registration> {
    if !trusted(root, true) {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut directories: Vec<_> = entries
        .take(64)
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    directories.sort();
    directories
        .iter()
        .filter_map(|directory| load(directory))
        .take(8)
        .collect()
}

pub(crate) fn config_root() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join("AppData").join("Roaming"))
        })
        .map(|path| path.join("Strata").join("providers"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_untrusted_external_provider_roots_on_windows() {
        assert!(config_root().is_some_and(|path| path.is_absolute()));
        assert!(discover(Path::new("C:\\untrusted\\providers")).is_empty());
    }
}
