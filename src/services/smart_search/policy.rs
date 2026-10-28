// SPDX-License-Identifier: MIT

use std::path::{Component, Path};

pub(super) fn excluded(path: &Path) -> bool {
    path.components()
        .filter_map(|component| {
            if let Component::Normal(value) = component {
                value.to_str()
            } else {
                None
            }
        })
        .any(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                ".git"
                    | ".ssh"
                    | ".gnupg"
                    | ".aws"
                    | ".azure"
                    | ".cache"
                    | ".cargo"
                    | ".rustup"
                    | "node_modules"
                    | "target"
                    | "dist"
                    | "build"
                    | "vendor"
                    | "__pycache__"
                    | ".venv"
                    | ".next"
                    | "appdata"
            )
        })
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                let name = name.to_ascii_lowercase();
                name == ".env"
                    || name.starts_with(".env.")
                    || matches!(
                        name.as_str(),
                        "credentials" | "id_rsa" | "id_ed25519" | "cachedir.tag"
                    )
                    || name.ends_with(".pem")
                    || name.ends_with(".key")
                    || name.ends_with(".pfx")
                    || name.ends_with(".p12")
            })
}

pub(super) fn sensitive(path: &Path) -> bool {
    path.components()
        .filter_map(|part| part.as_os_str().to_str())
        .any(|part| {
            matches!(
                part.to_ascii_lowercase().as_str(),
                ".ssh" | ".gnupg" | ".aws" | ".azure"
            ) || part.eq_ignore_ascii_case(".env")
                || part.to_ascii_lowercase().starts_with(".env.")
        })
}

pub(crate) fn normalize_path(path: &Path) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if let Some(tail) = raw.strip_prefix(r"\\?\UNC\") {
            return std::path::PathBuf::from(format!(r"\\{tail}"));
        }
        if let Some(tail) = raw.strip_prefix(r"\\?\") {
            return std::path::PathBuf::from(tail);
        }
    }
    path.to_path_buf()
}

pub(crate) fn within(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let normalize = |value: &Path| {
            let text = value.to_string_lossy().replace('/', r"\");
            if let Some(tail) = text.strip_prefix(r"\\?\UNC\") {
                format!(r"\\{tail}").to_lowercase()
            } else {
                text.trim_start_matches(r"\\?\").to_lowercase()
            }
        };
        let path = normalize(path);
        let root = normalize(root).trim_end_matches('\\').to_owned();
        path == root
            || path
                .strip_prefix(&root)
                .is_some_and(|tail| tail.starts_with('\\'))
    }
    #[cfg(not(windows))]
    {
        path.starts_with(root)
    }
}

pub(super) fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub(crate) fn code(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "rs" | "py"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "cs"
            | "java"
            | "go"
            | "swift"
            | "kt"
            | "rb"
            | "php"
            | "lua"
            | "sh"
            | "ps1"
            | "css"
            | "scss"
            | "sql"
            | "xml"
            | "ini"
            | "cfg"
    )
}

pub(super) fn image(path: &Path) -> bool {
    crate::sandbox::raw_metadata::is_raw(path)
        || matches!(
            extension(path).as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "avif"
        )
}

pub(super) fn media(path: &Path) -> Option<bool> {
    match extension(path).as_str() {
        "mp3" | "wav" | "flac" | "m4a" | "ogg" | "opus" | "aac" => Some(false),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" => Some(true),
        _ => None,
    }
}

pub(super) fn plain(path: &Path) -> bool {
    code(path)
        || matches!(
            extension(path).as_str(),
            "txt" | "md" | "markdown" | "log" | "csv" | "tsv" | "html" | "htm" | "rst"
        )
}

pub(super) fn cloud_placeholder(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & (0x1000 | 0x40000 | 0x400000) != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

#[cfg(test)]
mod tests;
