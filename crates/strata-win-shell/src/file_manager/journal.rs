// SPDX-License-Identifier: MIT

use super::{
    Registration,
    registry::{self, Value},
};
use std::{ffi::OsStr, io};
use windows::Win32::System::Registry::{REG_BINARY, REG_VALUE_TYPE};

pub(super) const NAME: &str = "JournalV2";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Change {
    pub path: String,
    pub name: String,
    pub original: Option<Value>,
    pub snapshot: Option<Value>,
    pub written: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Prepared,
    Active,
    Restoring,
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Journal {
    pub id: String,
    pub executable: Value,
    pub proxy: Value,
    pub label: Value,
    pub phase: Phase,
    pub changes: Vec<Change>,
    pub created_keys: Vec<String>,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid shell integration journal",
    )
}

fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len()).map_err(|_| invalid())?;
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(bytes);
    Ok(())
}

fn put_value(output: &mut Vec<u8>, value: &Option<Value>) -> io::Result<()> {
    output.extend_from_slice(&value.as_ref().map_or(u32::MAX, |v| v.kind.0).to_le_bytes());
    put_bytes(output, value.as_ref().map_or(&[], |v| &v.data))
}

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn number(&mut self) -> io::Result<u32> {
        if self.0.len() < 4 {
            return Err(invalid());
        }
        let result = u32::from_le_bytes(self.0[..4].try_into().map_err(|_| invalid())?);
        self.0 = &self.0[4..];
        Ok(result)
    }
    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let length = self.number()? as usize;
        if length > self.0.len() {
            return Err(invalid());
        }
        let (value, rest) = self.0.split_at(length);
        self.0 = rest;
        Ok(value)
    }
    fn string(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| invalid())
    }
    fn value(&mut self) -> io::Result<Option<Value>> {
        let kind = self.number()?;
        let data = self.bytes()?.to_vec();
        if kind == u32::MAX {
            if !data.is_empty() {
                return Err(invalid());
            }
            Ok(None)
        } else {
            Ok(Some(Value {
                kind: REG_VALUE_TYPE(kind),
                data,
            }))
        }
    }
}

impl Journal {
    pub fn new(registration: &Registration) -> io::Result<Self> {
        let executable = registration.metadata("Executable")?.ok_or_else(invalid)?;
        let proxy = registration.metadata("Proxy")?.ok_or_else(invalid)?;
        let label = registration
            .metadata("Label")?
            .or(registry::get(&registration.verb("Directory"), "")?)
            .ok_or_else(invalid)?;
        Self::with_identity(executable, proxy, label)
    }
    pub fn with_identity(executable: Value, proxy: Value, label: Value) -> io::Result<Self> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        Ok(Self {
            id: format!("{:x}-{stamp:x}", std::process::id()),
            executable,
            proxy,
            label,
            phase: Phase::Prepared,
            changes: Vec::new(),
            created_keys: Vec::new(),
        })
    }
    pub fn encode(&self) -> io::Result<Value> {
        let mut data = b"STRATA-SHELL\0\x02".to_vec();
        put_bytes(&mut data, self.id.as_bytes())?;
        for value in [&self.executable, &self.proxy, &self.label] {
            put_value(&mut data, &Some(value.clone()))?;
        }
        data.extend_from_slice(&(self.phase as u32).to_le_bytes());
        data.extend_from_slice(&(self.changes.len() as u32).to_le_bytes());
        for change in &self.changes {
            put_bytes(&mut data, change.path.as_bytes())?;
            put_bytes(&mut data, change.name.as_bytes())?;
            put_value(&mut data, &change.original)?;
            put_value(&mut data, &change.snapshot)?;
            put_value(&mut data, &change.written)?;
        }
        data.extend_from_slice(&(self.created_keys.len() as u32).to_le_bytes());
        for key in &self.created_keys {
            put_bytes(&mut data, key.as_bytes())?;
        }
        if data.len() > 65_536 {
            return Err(invalid());
        }
        Ok(Value {
            kind: REG_BINARY,
            data,
        })
    }

    fn decode(value: Value) -> io::Result<Self> {
        let prefix = b"STRATA-SHELL\0\x02";
        if value.kind != REG_BINARY || !value.data.starts_with(prefix) {
            return Err(invalid());
        }
        let mut reader = Reader(&value.data[prefix.len()..]);
        let id = reader.string()?;
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        {
            return Err(invalid());
        }
        let executable = reader.value()?.ok_or_else(invalid)?;
        let proxy = reader.value()?.ok_or_else(invalid)?;
        let label = reader.value()?.ok_or_else(invalid)?;
        for value in [&executable, &proxy] {
            if !value
                .os_string()
                .is_some_and(|path| std::path::Path::new(&path).is_absolute())
            {
                return Err(invalid());
            }
        }
        if label.os_string().is_none() {
            return Err(invalid());
        }
        let phase = match reader.number()? {
            0 => Phase::Prepared,
            1 => Phase::Active,
            2 => Phase::Restoring,
            3 => Phase::Conflict,
            _ => return Err(invalid()),
        };
        let count = reader.number()?;
        if count > 32 {
            return Err(invalid());
        }
        let mut changes = Vec::new();
        for _ in 0..count {
            changes.push(Change {
                path: reader.string()?,
                name: reader.string()?,
                original: reader.value()?,
                snapshot: reader.value()?,
                written: reader.value()?,
            });
        }
        let count = reader.number()?;
        if count > 64 {
            return Err(invalid());
        }
        let mut created_keys = Vec::new();
        for _ in 0..count {
            created_keys.push(reader.string()?);
        }
        if !reader.0.is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            id,
            executable,
            proxy,
            label,
            phase,
            changes,
            created_keys,
        })
    }

    pub fn read(registration: &Registration) -> io::Result<Option<Self>> {
        registry::get(&registration.state, NAME)?
            .map(Self::decode)
            .transpose()
    }
    pub fn load_if_present(registration: &Registration) -> io::Result<Option<Self>> {
        Self::read(registration)?
            .map(|journal| {
                let proxy = journal.proxy.os_string().ok_or_else(invalid)?;
                let allowed = registration
                    .routes_for(std::path::Path::new(&proxy), Some(journal.label.clone()))?;
                let mut seen = std::collections::HashSet::new();
                if journal.changes.is_empty() {
                    return Err(invalid());
                }
                for change in &journal.changes {
                    if !seen.insert((&change.path, &change.name)) {
                        return Err(invalid());
                    }
                    let legacy_run =
                        change.path == registration.run && change.name == super::RUN_VALUE;
                    if !legacy_run
                        && !allowed.iter().any(|candidate| {
                            candidate.path == change.path
                                && candidate.name == change.name
                                && candidate.written == change.written
                        })
                    {
                        return Err(invalid());
                    }
                }
                for key in &journal.created_keys {
                    if !allowed.iter().any(|change| {
                        change.path == *key || change.path.starts_with(&format!("{key}\\"))
                    }) {
                        return Err(invalid());
                    }
                    if key == &registration.classes
                        || !key.starts_with(&format!("{}\\", registration.classes))
                    {
                        return Err(invalid());
                    }
                }
                Ok(journal)
            })
            .transpose()
    }

    pub fn save(&self, registration: &Registration) -> io::Result<()> {
        if registry::get(&registration.state, "InstallationId")?.is_none() {
            registry::set(
                &registration.state,
                "InstallationId",
                Some(&Value::text(OsStr::new(&self.id))?),
            )?;
        }
        registry::set(&registration.state, NAME, Some(&self.encode()?))?;
        registry::flush(&registration.state)
    }

    pub fn restore(&mut self, registration: &Registration) -> io::Result<bool> {
        self.phase = Phase::Restoring;
        self.save(registration)?;
        let mut conflict = false;
        // The complete journal remains durable until every owned write has been restored.
        for change in self.changes.iter().rev() {
            let current = registry::get(&change.path, &change.name)?;
            if current == change.written || current == change.snapshot {
                registry::set(&change.path, &change.name, change.original.as_ref())?;
                registry::flush(&change.path)?;
            } else if current != change.original {
                conflict = true;
            }
        }
        for key in self.created_keys.iter().rev() {
            registry::delete_empty_key(key)?;
        }
        registration.retire_legacy()?;
        if conflict {
            self.phase = Phase::Conflict;
            self.save(registration)?;
        } else {
            for name in [
                NAME,
                "Executable",
                "Proxy",
                "Command",
                "Activation",
                "Directory",
                "Drive",
                "WatchCommand",
                "RunBackup",
                "Label",
                "Launching",
            ] {
                registry::set(&registration.state, name, None)?;
                registry::set(&registration.state, &format!("Legacy.{name}"), None)?;
            }
            registry::flush(&registration.state)?;
            registry::delete_empty_key(&registration.state)?;
        }
        Ok(conflict)
    }

    pub fn capture_keys(&mut self, path: &str, classes: &str) -> io::Result<()> {
        let mut end = classes.len();
        while end < path.len() {
            end = path[end + 1..]
                .find('\\')
                .map_or(path.len(), |index| end + 1 + index);
            let key = &path[..end];
            if !registry::key_exists(key)? && !self.created_keys.iter().any(|known| known == key) {
                self.created_keys.push(key.to_owned());
            }
        }
        Ok(())
    }
}

pub(super) fn text(value: &str) -> io::Result<Option<Value>> {
    Ok(Some(Value::text(OsStr::new(value))?))
}

#[cfg(test)]
pub(super) mod tests;
