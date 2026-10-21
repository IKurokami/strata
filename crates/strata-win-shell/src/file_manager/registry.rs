// SPDX-License-Identifier: MIT

use std::{
    ffi::{OsStr, OsString},
    io,
    os::windows::ffi::{OsStrExt, OsStringExt},
};
use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, WIN32_ERROR},
        System::Registry::*,
    },
    core::PCWSTR,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Value {
    pub kind: REG_VALUE_TYPE,
    pub data: Vec<u8>,
}

pub(super) fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut value: Vec<u16> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Embedded NUL"));
    }
    value.push(0);
    Ok(value)
}

impl Value {
    pub fn text(value: &OsStr) -> io::Result<Self> {
        Ok(Self {
            kind: REG_SZ,
            data: wide(value)?
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect(),
        })
    }

    pub fn os_string(&self) -> Option<OsString> {
        if self.kind != REG_SZ || self.data.len() % 2 != 0 {
            return None;
        }
        let mut value: Vec<u16> = self
            .data
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        if value.pop() != Some(0) || value.contains(&0) {
            return None;
        }
        Some(OsString::from_wide(&value))
    }

    pub fn backup(value: Option<Self>) -> Self {
        let mut data = Vec::new();
        let kind = value.as_ref().map_or(u32::MAX, |value| value.kind.0);
        data.extend_from_slice(&kind.to_le_bytes());
        if let Some(value) = value {
            data.extend_from_slice(&value.data);
        }
        Self {
            kind: REG_BINARY,
            data,
        }
    }

    pub fn original(&self) -> io::Result<Option<Self>> {
        if self.kind != REG_BINARY || self.data.len() < 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid file-manager backup",
            ));
        }
        let kind = u32::from_le_bytes(self.data[..4].try_into().expect("backup header"));
        Ok((kind != u32::MAX).then(|| Self {
            kind: REG_VALUE_TYPE(kind),
            data: self.data[4..].to_vec(),
        }))
    }
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: This wrapper owns the opened key, never a predefined hive.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn checked(result: WIN32_ERROR) -> io::Result<()> {
    result.ok().map_err(io::Error::other)
}

fn open(path: &str, create: bool) -> io::Result<Option<Key>> {
    let path = wide(OsStr::new(path))?;
    let mut key = HKEY::default();
    // SAFETY: Strings are terminated and the writable output is live. All
    // operations are scoped to the current user's explicitly named subkey.
    let result = unsafe {
        if create {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
                None,
                &mut key,
                None,
            )
        } else {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                None,
                KEY_READ | KEY_WOW64_64KEY,
                &mut key,
            )
        }
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    checked(result)?;
    Ok(Some(Key(key)))
}

pub(super) fn get(path: &str, name: &str) -> io::Result<Option<Value>> {
    let Some(key) = open(path, false)? else {
        return Ok(None);
    };
    let name = wide(OsStr::new(name))?;
    for _ in 0..3 {
        let mut length = 0;
        let mut kind = REG_NONE;
        // SAFETY: Only initialized scalar outputs are supplied for the size query.
        let result = unsafe {
            RegQueryValueExW(
                key.0,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                None,
                Some(&mut length),
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        checked(result)?;
        if length > 65_536 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Oversized registry value",
            ));
        }
        let mut data = vec![0; length as usize];
        // SAFETY: Windows receives the exact allocated buffer capacity and valid
        // scalar outputs; a concurrent value resize causes a bounded retry.
        let result = unsafe {
            RegQueryValueExW(
                key.0,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr()),
                Some(&mut length),
            )
        };
        if result == ERROR_MORE_DATA {
            continue;
        }
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        checked(result)?;
        data.truncate(length as usize);
        return Ok(Some(Value { kind, data }));
    }
    Err(io::Error::other("Registry value kept changing"))
}

pub(super) fn set(path: &str, name: &str, value: Option<&Value>) -> io::Result<()> {
    let key = open(path, true)?.ok_or_else(|| io::Error::other("Registry key unavailable"))?;
    let name = wide(OsStr::new(name))?;
    // SAFETY: The key is owned, the name terminated, and data borrowed for the
    // synchronous call only. Only the named value is changed.
    let result = unsafe {
        if let Some(value) = value {
            RegSetValueExW(
                key.0,
                PCWSTR(name.as_ptr()),
                None,
                value.kind,
                Some(&value.data),
            )
        } else {
            RegDeleteValueW(key.0, PCWSTR(name.as_ptr()))
        }
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    checked(result)
}

pub(super) fn remove_tree(path: &str) -> io::Result<()> {
    let path = wide(OsStr::new(path))?;
    // SAFETY: Callers pass only an owned registration/journal subkey, never
    // a hive or a shared Classes parent.
    let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    checked(result)
}

pub(super) fn flush(path: &str) -> io::Result<()> {
    if let Some(key) = open(path, false)? {
        // SAFETY: The owned, live key remains open through the synchronous flush.
        checked(unsafe { RegFlushKey(key.0) })?;
    }
    Ok(())
}
