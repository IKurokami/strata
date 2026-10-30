// SPDX-License-Identifier: MIT

use super::{
    Registration,
    journal::{Journal, Phase},
    registry, same_path,
};
use std::{
    io,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

fn regular(path: &Path) -> bool {
    path.is_absolute()
        && std::fs::symlink_metadata(path)
            .is_ok_and(|m| m.is_file() && m.file_attributes() & 0x400 == 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actual {
    Disabled,
    Active,
    Partial,
    Conflict,
    Broken,
    Unsupported,
}
impl Actual {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Active => "active",
            Self::Partial => "partial",
            Self::Conflict => "conflict",
            Self::Broken => "broken",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteStatus {
    Owned,
    Absent,
    Conflict,
}
impl RouteStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owned => "owned",
            Self::Absent => "absent",
            Self::Conflict => "conflict",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegrationStatus {
    pub actual: Actual,
    pub transaction_id: Option<String>,
    pub registration_version: Option<u32>,
    pub owner_executable_exists: bool,
    pub proxy_exists: bool,
    pub folder_open: RouteStatus,
    pub folder_explore: RouteStatus,
    pub win_e_clsid: RouteStatus,
    pub drive_open: RouteStatus,
    pub legacy_watcher_running: bool,
    pub repairable: bool,
}

impl IntegrationStatus {
    pub fn disabled() -> Self {
        Self {
            actual: Actual::Disabled,
            transaction_id: None,
            registration_version: None,
            owner_executable_exists: false,
            proxy_exists: false,
            folder_open: RouteStatus::Absent,
            folder_explore: RouteStatus::Absent,
            win_e_clsid: RouteStatus::Absent,
            drive_open: RouteStatus::Absent,
            legacy_watcher_running: false,
            repairable: false,
        }
    }
}

impl Registration {
    pub(super) fn probe(&self, executable: Option<&Path>) -> io::Result<IntegrationStatus> {
        let mut status = IntegrationStatus::disabled();
        let owner = self.owner()?;
        let proxy = self
            .metadata("Proxy")?
            .and_then(|value| value.os_string())
            .map(PathBuf::from);
        status.owner_executable_exists = owner.as_ref().is_some_and(|p| regular(p));
        status.proxy_exists = proxy.as_ref().is_some_and(|p| regular(p));
        if owner.is_none() {
            let paths = [
                r"Folder\shell\open\command",
                r"Folder\shell\explore\command",
                r"CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command",
            ];
            for path in paths {
                let path = format!(r"{}\{path}", self.classes);
                if registry::get(&path, "")?.is_some()
                    || registry::get(&path, "DelegateExecute")?.is_some()
                {
                    status.actual = Actual::Conflict;
                }
            }
            return Ok(status);
        }
        if let (Some(executable), Some(owner)) = (executable, owner.as_ref()) {
            if !same_path(executable, owner) {
                status.actual = Actual::Conflict;
                return Ok(status);
            }
        }
        let journal = Journal::load_if_present(self)?;
        status.registration_version = Some(if journal.is_some() { 2 } else { 1 });
        let Some(journal) = journal else {
            status.actual = Actual::Partial;
            status.repairable = status.owner_executable_exists;
            return Ok(status);
        };
        status.transaction_id = Some(journal.id.clone());
        let proxy = journal
            .proxy
            .os_string()
            .ok_or_else(|| io::Error::other("Missing journal proxy"))?;
        let routes = self.routes_for(Path::new(&proxy), Some(journal.label.clone()))?;
        let probe = |start: usize, count: usize| -> io::Result<RouteStatus> {
            let mut absent = true;
            let mut matches = true;
            for route in &routes[start..start + count] {
                let value = registry::get(&route.path, &route.name)?;
                absent &= value.is_none();
                matches &= value == route.written;
                if self.classes == r"Software\Classes" {
                    let suffix = route
                        .path
                        .strip_prefix(r"Software\Classes\")
                        .expect("known class prefix");
                    matches &= registry::effective_get(suffix, &route.name)? == route.written;
                }
            }
            Ok(if matches {
                RouteStatus::Owned
            } else if absent {
                RouteStatus::Absent
            } else {
                RouteStatus::Conflict
            })
        };
        status.folder_open = probe(0, 2)?;
        status.folder_explore = probe(2, 2)?;
        status.win_e_clsid = probe(4, 2)?;
        status.drive_open = probe(9, 3)?;
        let all_owned = [
            status.folder_open,
            status.folder_explore,
            status.win_e_clsid,
            status.drive_open,
            probe(6, 3)?,
        ]
        .into_iter()
        .all(|state| state == RouteStatus::Owned);
        let metadata_matches = registry::get(&self.state, "Executable")?.as_ref()
            == Some(&journal.executable)
            && registry::get(&self.state, "Proxy")?.as_ref() == Some(&journal.proxy)
            && registry::get(&self.state, "Label")?.as_ref() == Some(&journal.label);
        status.actual =
            if !status.owner_executable_exists || !status.proxy_exists || !metadata_matches {
                Actual::Broken
            } else if journal.phase == Phase::Conflict {
                Actual::Conflict
            } else if all_owned && journal.phase == Phase::Active {
                Actual::Active
            } else if [
                status.folder_open,
                status.folder_explore,
                status.win_e_clsid,
                status.drive_open,
            ]
            .contains(&RouteStatus::Conflict)
            {
                Actual::Conflict
            } else {
                Actual::Partial
            };
        status.repairable = status.owner_executable_exists && status.actual != Actual::Conflict;
        Ok(status)
    }
}
