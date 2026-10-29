// SPDX-License-Identifier: MIT

use std::{borrow::Cow, cell::RefCell, collections::HashMap, path::Path};

use gtk::gio;

use crate::model::{EntryKind, FileEntry};

pub const FOLDER_TYPE_NAME: &str = "Folder";
pub const BROKEN_LINK_TYPE_NAME: &str = "Broken link";
pub const OTHER_TYPE_NAME: &str = "Other";

const TYPE_CACHE_LIMIT: usize = 2048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntryType {
    Folder,
    BrokenLink,
    Known(String),
    Other,
}

impl EntryType {
    pub fn description(&self) -> &str {
        match self {
            Self::Folder => FOLDER_TYPE_NAME,
            Self::BrokenLink => BROKEN_LINK_TYPE_NAME,
            Self::Known(description) => description.as_str(),
            Self::Other => OTHER_TYPE_NAME,
        }
    }
}

pub fn entry_type(entry: &FileEntry) -> EntryType {
    if entry.is_directory() {
        EntryType::Folder
    } else if entry.is_broken_symbolic_link() {
        EntryType::BrokenLink
    } else if entry.kind == EntryKind::Other {
        EntryType::Other
    } else {
        mime_type_for_name(&entry.display_name)
    }
}

pub fn mime_type_for_name(name: &str) -> EntryType {
    let description = mime_description_for_name(name);
    if description == OTHER_TYPE_NAME {
        EntryType::Other
    } else {
        EntryType::Known(description)
    }
}

pub fn mime_description_for_name(name: &str) -> String {
    TYPE_CACHE.with_borrow_mut(|cache| {
        // MIME globs can match compound suffixes and whole names, not just extensions.
        #[cfg(not(windows))]
        let key: Cow<'_, str> = Cow::Borrowed(name);
        #[cfg(windows)]
        let key: Cow<'_, str> = Path::new(name).extension().map_or_else(
            || Cow::Borrowed(name),
            |extension| Cow::Owned(format!(".{}", extension.to_string_lossy().to_lowercase())),
        );
        if let Some(description) = cache.get(key.as_ref()) {
            return description.clone();
        }
        let description = guess_mime_description(name);
        if cache.len() >= TYPE_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key.into_owned(), description.clone());
        description
    })
}

fn guess_mime_description(name: &str) -> String {
    let (content_type, _) = gio::content_type_guess(Some(Path::new(name)), None::<&[u8]>);
    if content_type.is_empty() || content_type == "application/octet-stream" {
        return OTHER_TYPE_NAME.to_owned();
    }
    #[cfg(windows)]
    if gio::content_type_get_mime_type(&content_type).is_none_or(|mime| {
        mime.starts_with("application/x-ext-") || mime == "application/octet-stream"
    }) && !strata_win_shell::launch::has_registered_type(Path::new(name))
    {
        return OTHER_TYPE_NAME.to_owned();
    }
    let description = gio::content_type_get_description(&content_type);
    if description.is_empty() {
        return OTHER_TYPE_NAME.to_owned();
    }
    description.to_string()
}

thread_local! {
    static TYPE_CACHE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

#[cfg(test)]
mod tests;
