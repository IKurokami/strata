// SPDX-License-Identifier: MIT

use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchExclusions {
    folder_names: Vec<String>,
    directories: Vec<PathBuf>,
}

impl SearchExclusions {
    pub fn is_directory_path(raw: &str) -> bool {
        raw.starts_with('~')
            || raw.contains('/')
            || (cfg!(windows) && (raw.contains('\\') || raw.as_bytes().get(1) == Some(&b':')))
    }

    pub fn normalize_entry(raw: &str) -> Result<String, &'static str> {
        let input = raw.trim();
        if input.is_empty() {
            return Err("Enter a folder name or directory path.");
        }
        if input.contains('\0') {
            return Err("Exclusions cannot contain NUL characters.");
        }
        if Path::new(input).is_absolute() && Path::new(input).file_name().is_none() {
            return Err("Cannot exclude root or entire home directory.");
        }
        let trimmed = input.trim_end_matches(|c| c == '/' || (cfg!(windows) && c == '\\'));
        if trimmed.is_empty() || trimmed == "~" {
            return Err("Cannot exclude root or entire home directory.");
        }
        if !Self::is_directory_path(trimmed) {
            if matches!(trimmed, "." | "..") {
                return Err("Enter a folder name, not . or ..");
            }
            return Ok(trimmed.to_owned());
        }
        let home_relative = trimmed.strip_prefix("~/").or_else(|| {
            if cfg!(windows) {
                trimmed.strip_prefix("~\\")
            } else {
                None
            }
        });
        let path = if let Some(rest) = home_relative {
            glib::home_dir()
                .join(rest.trim_start_matches(|c| c == '/' || (cfg!(windows) && c == '\\')))
        } else if Path::new(trimmed).is_absolute() {
            PathBuf::from(trimmed)
        } else {
            return Err("Directory paths must be absolute or start with ~/.");
        };
        // Collapsing .. lexically would change the meaning of paths through symlinks.
        if path.components().any(|part| part == Component::ParentDir) {
            return Err("Directory paths cannot contain ..");
        }
        let path: PathBuf = path.components().collect();
        let home: PathBuf = glib::home_dir().components().collect();
        if path.file_name().is_none()
            || path == home
            || (cfg!(windows)
                && path
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&home.to_string_lossy()))
        {
            return Err("Cannot exclude root or entire home directory.");
        }
        Ok(path.to_string_lossy().into_owned())
    }

    pub fn from_strings(raw: &[String]) -> Self {
        let mut exclusions = Self::default();
        for item in raw {
            let Ok(normalized) = Self::normalize_entry(item) else {
                continue;
            };
            if Path::new(&normalized).is_absolute() {
                exclusions.directories.push(PathBuf::from(normalized));
            } else {
                exclusions
                    .folder_names
                    .push(normalized.to_ascii_lowercase());
            }
        }
        exclusions.folder_names.sort();
        exclusions.folder_names.dedup();
        exclusions.directories.sort();
        exclusions.directories.dedup();
        exclusions
    }

    pub fn is_excluded(&self, path: &Path, name: &str, is_directory: bool) -> bool {
        self.directories
            .iter()
            .any(|dir| directory_contains(dir, path))
            || (is_directory && self.matches_folder(name))
            || (!self.folder_names.is_empty()
                && path.ancestors().skip(1).any(|parent| {
                    parent
                        .file_name()
                        .is_some_and(|name| self.matches_folder(&name.to_string_lossy()))
                }))
    }

    fn matches_folder(&self, name: &str) -> bool {
        self.folder_names
            .iter()
            .any(|folder| name.eq_ignore_ascii_case(folder))
    }
}

fn directory_contains(directory: &Path, path: &Path) -> bool {
    #[cfg(windows)]
    {
        let mut components = path.components();
        directory.components().all(|part| {
            components.next().is_some_and(|candidate| {
                part.as_os_str().to_string_lossy().to_lowercase()
                    == candidate.as_os_str().to_string_lossy().to_lowercase()
            })
        })
    }
    #[cfg(not(windows))]
    path.starts_with(directory)
}

#[cfg(test)]
mod tests;
