// SPDX-License-Identifier: MIT

use super::{
    Actual, CLASSES, RUN_VALUE, Registration, VERB,
    journal::{self, Change, Journal, Phase},
    registry::{self, Value},
    same_path,
};
use std::{
    ffi::{OsStr, OsString},
    io,
    path::Path,
};

const WIN_E: &str = r"CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command";

fn conflict() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Another application changed the file-manager registration",
    )
}

pub(super) fn command(proxy: &Path, activate: bool) -> io::Result<Value> {
    if !proxy.is_absolute()
        || proxy
            .as_os_str()
            .encode_wide()
            .any(|c| c == 0 || c == u16::from(b'"'))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid shell proxy path",
        ));
    }
    let mut value = OsString::from("\"");
    value.push(proxy);
    // A trailing drive separator must not escape the closing Windows argv quote.
    value.push(if activate {
        "\" --activate"
    } else {
        "\" --open \"%1\\.\""
    });
    Value::text(&value)
}
use std::os::windows::ffi::OsStrExt;

impl Registration {
    pub(super) fn owns_any_route(&self) -> io::Result<bool> {
        let proxy = super::proxy_path()?;
        for suffix in [
            r"Folder\shell\open\command",
            r"Folder\shell\explore\command",
            WIN_E,
            r"Directory\shell\Strata.Open\command",
            r"Drive\shell\Strata.Open\command",
        ] {
            let value = registry::get(&format!(r"{}\{suffix}", self.classes), "")?;
            for executable in [&proxy, &proxy.with_file_name("strata-shell.exe")] {
                if value.as_ref() == Some(&command(executable, suffix == WIN_E)?) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    pub(super) fn routes_for(&self, proxy: &Path, label: Option<Value>) -> io::Result<Vec<Change>> {
        let open = Some(command(proxy, false)?);
        let activate = Some(command(proxy, true)?);
        let mut routes = Vec::new();
        for suffix in [
            r"Folder\shell\open\command",
            r"Folder\shell\explore\command",
            WIN_E,
        ] {
            let path = format!(r"{}\{suffix}", self.classes);
            routes.push(Change {
                path: path.clone(),
                name: String::new(),
                original: None,
                snapshot: None,
                written: if suffix == WIN_E {
                    activate.clone()
                } else {
                    open.clone()
                },
            });
            routes.push(Change {
                path,
                name: "DelegateExecute".into(),
                original: None,
                snapshot: None,
                written: journal::text("")?,
            });
        }
        for class in CLASSES {
            routes.push(Change {
                path: self.shell(class),
                name: String::new(),
                original: None,
                snapshot: None,
                written: journal::text(VERB)?,
            });
            routes.push(Change {
                path: self.verb(class),
                name: String::new(),
                original: None,
                snapshot: None,
                written: label.clone(),
            });
            routes.push(Change {
                path: format!(r"{}\command", self.verb(class)),
                name: String::new(),
                original: None,
                snapshot: None,
                written: open.clone(),
            });
        }
        Ok(routes)
    }

    pub(super) fn enabled(&self, executable: &Path) -> io::Result<bool> {
        Ok(self.probe(Some(executable))?.actual == Actual::Active)
    }

    pub(super) fn enable(&self, executable: &Path, proxy: &Path, label: &str) -> io::Result<()> {
        self.enable_policy(executable, proxy, label, true)
    }

    pub(super) fn enable_policy(
        &self,
        executable: &Path,
        proxy: &Path,
        label: &str,
        replace_foreign: bool,
    ) -> io::Result<()> {
        if !executable.is_absolute() || !executable.is_file() || !proxy.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Missing shell integration executable",
            ));
        }
        let expected = command(proxy, false)?;
        let requested_label = Value::text(OsStr::new(label))?;
        if let Some(owner) = self.owner()?
            && !same_path(&owner, executable)
        {
            if !replace_foreign {
                return Err(conflict());
            }
            self.disable(&owner)?;
        }
        let previous = Journal::load_if_present(self)?;
        let previously_v2 = previous.is_some();
        let mut replacing_owned = false;
        if !previously_v2 && self.owner()?.is_some() && self.metadata("WatchCommand")?.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing integration journal",
            ));
        }
        if let Some(mut journal) = previous {
            let mut routes_owned = true;
            for change in journal
                .changes
                .iter()
                .filter(|change| change.path != self.run)
            {
                routes_owned &= registry::get(&change.path, &change.name)? == change.written;
            }
            let same_registration = journal
                .executable
                .os_string()
                .is_some_and(|owner| same_path(Path::new(&owner), executable))
                && journal
                    .proxy
                    .os_string()
                    .is_some_and(|owner| same_path(Path::new(&owner), proxy))
                && journal.label == requested_label;
            if journal.phase == Phase::Active && routes_owned && same_registration {
                for (name, value) in [
                    ("Executable", &journal.executable),
                    ("Proxy", &journal.proxy),
                    ("Label", &journal.label),
                ] {
                    if registry::get(&self.state, name)?.as_ref() != Some(value) {
                        registry::set(&self.state, name, Some(value))?;
                    }
                }
                registry::flush(&self.state)?;
            }
            if journal.phase == Phase::Active && same_registration && self.enabled(executable)? {
                return Ok(());
            }
            if journal.phase == Phase::Active && !replace_foreign && !routes_owned {
                return Err(conflict());
            }
            if journal.phase == Phase::Conflict && !replace_foreign {
                return Err(conflict());
            }
            replacing_owned = journal.phase == Phase::Active && routes_owned;
            journal.restore(self)?;
            // Explicit takeover starts a new transaction only after old owned writes were restored.
            registry::set(&self.state, journal::NAME, None)?;
        }
        self.import_legacy()?;
        let legacy = !previously_v2 && self.metadata("WatchCommand")?.is_some();
        let legacy_command = self.command()?;
        for class in CLASSES {
            let existing = registry::get(&format!(r"{}\command", self.verb(class)), "")?;
            if existing.is_some() && (!legacy || existing != legacy_command) {
                return Err(conflict());
            }
        }
        // New leaf overrides must not silently replace another manager at startup.
        if !replace_foreign && !replacing_owned {
            for suffix in [
                r"Folder\shell\open\command",
                r"Folder\shell\explore\command",
                WIN_E,
            ] {
                let path = format!(r"{}\{suffix}", self.classes);
                if registry::get(&path, "")?.is_some()
                    || registry::get(&path, "DelegateExecute")?.is_some()
                {
                    return Err(conflict());
                }
            }
            for class in CLASSES {
                if registry::get(&self.shell(class), "")?.is_some_and(|v| {
                    v.os_string().is_none_or(|v| {
                        !v.is_empty()
                            && v != "none"
                            && v != "open"
                            && v != "explore"
                            && !(legacy && v == VERB)
                    })
                }) {
                    return Err(conflict());
                }
            }
        }
        let label = requested_label;
        let mut journal = Journal::with_identity(
            Value::text(executable.as_os_str())?,
            Value::text(proxy.as_os_str())?,
            label.clone(),
        )?;
        let mut routes = self.routes_for(proxy, Some(label))?;
        let original_run = self
            .metadata("RunBackup")?
            .map(|v| v.original())
            .transpose()?
            .flatten();
        routes.push(Change {
            path: self.run.clone(),
            name: RUN_VALUE.into(),
            original: None,
            snapshot: None,
            written: original_run,
        });
        for mut change in routes {
            change.snapshot = registry::get(&change.path, &change.name)?;
            change.original = change.snapshot.clone();
            if change.path == self.run {
                if !legacy || change.original != self.metadata("WatchCommand")? {
                    continue;
                }
                change.original = self
                    .metadata("RunBackup")?
                    .ok_or_else(|| io::Error::other("Missing legacy startup backup"))?
                    .original()?;
            } else if legacy {
                for class in CLASSES {
                    if change.path == self.shell(class) {
                        if change.original != journal::text(VERB)? {
                            return Err(conflict());
                        }
                        change.original = registry::get(&self.state, class)?
                            .ok_or_else(|| io::Error::other("Missing legacy folder backup"))?
                            .original()?;
                    } else if change.path == self.verb(class)
                        || change.path == format!(r"{}\command", self.verb(class))
                    {
                        change.original = None;
                    }
                }
            }
            if change.path.starts_with(&format!("{}\\", self.classes)) {
                journal.capture_keys(&change.path, &self.classes)?;
            }
            journal.changes.push(change);
        }
        journal.save(self)?;
        let outcome = (|| {
            for (name, value) in [
                ("Executable", &journal.executable),
                ("Proxy", &journal.proxy),
                ("Label", &journal.label),
                ("Command", &expected),
            ] {
                registry::set(&self.state, name, Some(value))?;
            }
            registry::flush(&self.state)?;
            for change in &journal.changes {
                if registry::get(&change.path, &change.name)? != change.snapshot {
                    return Err(conflict());
                }
                registry::set(&change.path, &change.name, change.written.as_ref())?;
                registry::flush(&change.path)?;
                #[cfg(test)]
                journal::tests::after_write()?;
                if registry::get(&change.path, &change.name)? != change.written {
                    return Err(conflict());
                }
            }
            journal.phase = Phase::Active;
            journal.save(self)
        })();
        if outcome.is_err() {
            let _ = journal.restore(self);
        }
        outcome
    }

    pub(super) fn disable(&self, executable: &Path) -> io::Result<()> {
        if !self
            .owner()?
            .is_some_and(|owner| same_path(&owner, executable))
        {
            return Ok(());
        }
        self.import_legacy()?;
        if let Some(mut journal) = Journal::load_if_present(self)? {
            journal.restore(self)?;
            return Ok(());
        }
        // Migrate a v1 rollback journal without writing any v2 handlers.
        if self.metadata("WatchCommand")?.is_some() {
            let mut journal = Journal::new(self)?;
            let old_command = self.command()?;
            for class in CLASSES {
                let original = registry::get(&self.state, class)?
                    .ok_or_else(|| io::Error::other("Missing legacy backup"))?
                    .original()?;
                journal.changes.push(Change {
                    path: self.shell(class),
                    name: String::new(),
                    snapshot: registry::get(&self.shell(class), "")?,
                    original,
                    written: journal::text(VERB)?,
                });
                journal.changes.push(Change {
                    path: format!(r"{}\command", self.verb(class)),
                    name: String::new(),
                    original: None,
                    snapshot: old_command.clone(),
                    written: old_command.clone(),
                });
            }
            if registry::get(&self.run, RUN_VALUE)? == self.metadata("WatchCommand")? {
                let backup = self
                    .metadata("RunBackup")?
                    .ok_or_else(|| io::Error::other("Missing legacy startup backup"))?
                    .original()?;
                let old = self.metadata("WatchCommand")?;
                journal.changes.push(Change {
                    path: self.run.clone(),
                    name: RUN_VALUE.into(),
                    original: backup,
                    snapshot: old.clone(),
                    written: old,
                });
            }
            journal.save(self)?;
            journal.restore(self)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing integration journal",
            ));
        }
        Ok(())
    }
}
