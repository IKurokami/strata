// SPDX-License-Identifier: MIT
use std::{io::Read, path::Path};

pub(super) fn read(path: &Path) -> Option<bool> {
    let mut content = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(1024 * 1024 + 1)
        .read_to_string(&mut content)
        .ok()?;
    if content.len() > 1024 * 1024 {
        return None;
    }
    let value: toml::Value = toml::from_str(&content).ok()?;
    value.get("replace_file_explorer")?.as_bool()
}

#[cfg(test)]
#[path = "preferences/tests.rs"]
mod tests;
