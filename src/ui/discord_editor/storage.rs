// SPDX-License-Identifier: MIT

use super::model::BYTE_LIMIT;
use gtk::{gio, glib, prelude::*};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub(super) struct FileSnapshot {
    pub path: PathBuf,
    pub text: String,
    bytes: Vec<u8>,
    etag: String,
    bom: bool,
    crlf: bool,
}

fn error(message: &str) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Failed, message)
}

fn read(path: &Path) -> Result<FileSnapshot, glib::Error> {
    let file = gio::File::for_path(path);
    let info = file.query_info(
        "standard::type,etag::value",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?;
    if info.file_type() != gio::FileType::Regular {
        return Err(error("Only regular text files can be edited"));
    }
    let etag = info
        .etag()
        .ok_or_else(|| error("Unable to verify the file version"))?
        .to_string();
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(BYTE_LIMIT as u64 + 1).read_to_end(&mut bytes))
        .map_err(|_| error("Unable to read this file"))?;
    if bytes.len() > BYTE_LIMIT {
        return Err(error("Text is too large for Discord editing"));
    }
    let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    let source = std::str::from_utf8(if bom { &bytes[3..] } else { &bytes })
        .map_err(|_| error("Discord editing requires UTF-8 text"))?;
    if source.contains('\0') {
        return Err(error("Discord editing requires UTF-8 text"));
    }
    let crlf = source.contains("\r\n");
    let text = source.replace("\r\n", "\n");
    let after = file.query_info(
        "etag::value",
        gio::FileQueryInfoFlags::NONE,
        gio::Cancellable::NONE,
    )?;
    if after.etag().as_deref() != Some(etag.as_str()) {
        return Err(error("The file changed outside the editor"));
    }
    Ok(FileSnapshot {
        path: path.to_path_buf(),
        text,
        bytes,
        etag,
        bom,
        crlf,
    })
}

pub(super) async fn load(path: PathBuf) -> Result<FileSnapshot, glib::Error> {
    gio::spawn_blocking(move || read(&path))
        .await
        .map_err(|_| error("Unable to read this file"))?
}

fn write(snapshot: FileSnapshot, text: String) -> Result<FileSnapshot, glib::Error> {
    let current = read(&snapshot.path)?;
    if current.bytes != snapshot.bytes || current.etag != snapshot.etag {
        return Err(error("The file changed outside the editor"));
    }
    let text = if snapshot.crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    };
    let mut bytes = if snapshot.bom {
        vec![0xef, 0xbb, 0xbf]
    } else {
        Vec::new()
    };
    bytes.extend_from_slice(text.as_bytes());
    if bytes.len() > BYTE_LIMIT {
        return Err(error("Text is too large for Discord editing"));
    }
    gio::File::for_path(&snapshot.path).replace_contents(
        &bytes,
        Some(&snapshot.etag),
        false,
        gio::FileCreateFlags::NONE,
        gio::Cancellable::NONE,
    )?;
    let saved = read(&snapshot.path)?;
    if saved.bytes != bytes {
        return Err(error("The file changed outside the editor"));
    }
    Ok(saved)
}

pub(super) async fn save(
    snapshot: FileSnapshot,
    text: String,
) -> Result<FileSnapshot, glib::Error> {
    gio::spawn_blocking(move || write(snapshot, text))
        .await
        .map_err(|_| error("Unable to save this file"))?
}

#[cfg(test)]
mod tests;
