// SPDX-License-Identifier: MIT
//! Win32 opt-in external file providers — disabled pending a contained native
//! provider process protocol. Do not reuse UnixStream, setsid, PATH lookup or
//! spawn an uncontained helper from a background selection refresh.
use std::{collections::BTreeMap, path::{Path, PathBuf}};
use serde::Deserialize;

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

/// The wire protocol needs the exact Unix slug rules, even though Win32
/// registration is disabled for now.
fn slug(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text.bytes().all(|ch| ch.is_ascii_alphanumeric() || ch == b'-')
}

/// No startup disk enumeration, symlink/ACL preflight, socket or process.
/// Return zero installed providers until Windows ACL ownership validation,
/// native pipe transport and process-tree containment exist.
pub(crate) fn discover(_root: &Path) -> Vec<Registration> {
    Vec::new()
}

pub(crate) fn config_root() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_external_provider_can_be_discovered_on_windows() {
        assert!(config_root().is_none());
        assert!(discover(Path::new("C:\\untrusted\\providers")).is_empty());
    }
}
