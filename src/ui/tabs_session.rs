// SPDX-License-Identifier: MIT

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::model::Location;

const SESSION_VERSION: u32 = 1;
const SESSION_FILE_NAME: &str = "tabs.toml";
const MAX_RESTORED_TABS: usize = 32;
const KIND_PATH: &str = "path";
const KIND_URI: &str = "uri";

enum WriteCommand {
    Save(PathBuf, Vec<Location>, usize),
    Remove(PathBuf),
    Flush(std::sync::mpsc::Sender<()>),
}

fn writer() -> Option<&'static std::sync::mpsc::Sender<WriteCommand>> {
    static WRITER: std::sync::OnceLock<Option<std::sync::mpsc::Sender<WriteCommand>>> =
        std::sync::OnceLock::new();
    WRITER
        .get_or_init(|| {
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("strata-tab-session".into())
                .spawn(move || write_sessions(receive))
                .ok()
                .map(|_| send)
        })
        .as_ref()
}

fn write_sessions(receive: std::sync::mpsc::Receiver<WriteCommand>) {
    let mut pending = None;
    loop {
        let next = if pending.is_some() {
            receive.recv_timeout(std::time::Duration::from_millis(150))
        } else {
            receive
                .recv()
                .map_err(|_| std::sync::mpsc::RecvTimeoutError::Disconnected)
        };
        match next {
            Ok(WriteCommand::Save(path, tabs, active)) => pending = Some((path, tabs, active)),
            Ok(WriteCommand::Remove(path)) => {
                pending = None;
                if let Err(error) = fs::remove_file(path)
                    && error.kind() != io::ErrorKind::NotFound
                {
                    tracing::warn!(%error, "unable to clear saved tabs");
                }
            }
            Ok(WriteCommand::Flush(done)) => {
                write_pending(&mut pending);
                let _ = done.send(());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => write_pending(&mut pending),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                write_pending(&mut pending);
                break;
            }
        }
    }
}

fn write_pending(pending: &mut Option<(PathBuf, Vec<Location>, usize)>) {
    if let Some((path, tabs, active)) = pending.take()
        && let Err(error) = save_to(&path, &tabs, active)
    {
        tracing::warn!(%error, "unable to save open tabs");
    }
}

pub(crate) fn save_deferred(tabs: &[Location], active: usize) {
    if writer().is_none_or(|writer| {
        writer
            .send(WriteCommand::Save(session_path(), tabs.to_vec(), active))
            .is_err()
    }) {
        save(tabs, active);
    }
}

pub(crate) fn flush() {
    let (send, receive) = std::sync::mpsc::channel();
    if writer().is_some_and(|writer| writer.send(WriteCommand::Flush(send)).is_ok()) {
        let _ = receive.recv_timeout(std::time::Duration::from_secs(3));
    }
}

const RESTORABLE_URI_SCHEMES: [&str; 9] = [
    "smb", "sftp", "ftp", "ftps", "dav", "davs", "trash", "network", "recent",
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StoredTab {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct StoredSession {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    active: usize,
    #[serde(default)]
    tabs: Vec<StoredTab>,
}

pub(crate) struct RestoredSession {
    pub tabs: Vec<Location>,
    pub active: usize,
}

#[cfg(any(windows, test))]
pub(crate) struct StartupSession {
    pub session: Option<RestoredSession>,
    pub fallback: Option<PathBuf>,
    pub invalid_default: Option<PathBuf>,
}

#[cfg(any(windows, test))]
impl StartupSession {
    pub fn fallback(default_directory: Option<PathBuf>) -> Self {
        let mut invalid_default = None;
        let fallback = default_directory
            .filter(|path| {
                if path.is_dir() {
                    true
                } else {
                    invalid_default = Some(path.clone());
                    false
                }
            })
            .unwrap_or_else(super::window::home_directory);
        Self {
            session: None,
            fallback: Some(fallback),
            invalid_default,
        }
    }
}

pub(crate) fn session_path() -> PathBuf {
    crate::storage::config_directory().join(SESSION_FILE_NAME)
}

pub(crate) fn save(tabs: &[Location], active: usize) {
    if let Err(error) = save_to(&session_path(), tabs, active) {
        tracing::warn!(%error, "unable to save open tabs");
    }
}

pub(crate) fn remove() {
    remove_deferred();
    flush();
}

pub(crate) fn remove_deferred() {
    if writer().is_none_or(|writer| writer.send(WriteCommand::Remove(session_path())).is_err()) {
        let _ = fs::remove_file(session_path());
    }
}

pub(crate) fn load_restorable() -> Option<RestoredSession> {
    load_from(&session_path())
}

pub(crate) fn save_to(path: &Path, tabs: &[Location], active: usize) -> io::Result<()> {
    let mut stored = Vec::new();
    let mut stored_active = 0;
    for (index, location) in tabs.iter().enumerate() {
        let entry = if let Some(path) = location.native_path() {
            path.to_str()
                .filter(|_| path.is_absolute())
                .map(|value| StoredTab::from(KIND_PATH, value))
        } else {
            location
                .uri_value()
                .and_then(|value| restorable_uri(value).map(|_| StoredTab::from(KIND_URI, value)))
        };
        if let Some(entry) = entry {
            if index <= active {
                stored_active = stored.len();
            }
            stored.push(entry);
        }
    }
    let session = StoredSession {
        version: SESSION_VERSION,
        active: stored_active,
        tabs: stored,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::storage::atomic_write(
        path,
        toml::to_string(&session)
            .map_err(io::Error::other)?
            .as_bytes(),
    )
}

pub(crate) fn load_from(path: &Path) -> Option<RestoredSession> {
    let contents = fs::read_to_string(path).ok()?;
    let stored: StoredSession = toml::from_str(&contents).ok()?;
    if stored.version != SESSION_VERSION {
        return None;
    }
    let mut tabs = Vec::new();
    let mut active = 0;
    for (index, tab) in stored.tabs.iter().enumerate() {
        if let Some(location) = restorable_location(&tab.kind, &tab.value) {
            if index <= stored.active {
                active = tabs.len();
            }
            tabs.push(location);
            if tabs.len() == MAX_RESTORED_TABS {
                break;
            }
        }
    }
    if tabs.is_empty() {
        return None;
    }
    Some(RestoredSession { active, tabs })
}

impl StoredTab {
    fn from(kind: &str, value: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            value: value.to_owned(),
        }
    }
}

fn restorable_location(kind: &str, value: &str) -> Option<Location> {
    match kind {
        KIND_PATH => {
            let path = PathBuf::from(value);
            if !path.is_absolute() || !path.is_dir() {
                return None;
            }
            Some(Location::local(path))
        }
        KIND_URI => restorable_uri(value),
        _ => None,
    }
}

fn restorable_uri(value: &str) -> Option<Location> {
    let uri = gio::glib::Uri::parse(
        value,
        gio::glib::UriFlags::HAS_PASSWORD | gio::glib::UriFlags::HAS_AUTH_PARAMS,
    )
    .ok()?;
    let scheme = uri.scheme().to_ascii_lowercase();
    if !RESTORABLE_URI_SCHEMES.contains(&scheme.as_str())
        || crate::model::uri_contains_credentials(&uri)
    {
        return None;
    }
    match scheme.as_str() {
        "trash" if value != "trash:///" => return None,
        "recent" if value != "recent:///" => return None,
        _ => {}
    }
    Some(Location::uri(value))
}

#[cfg(test)]
mod tests;
