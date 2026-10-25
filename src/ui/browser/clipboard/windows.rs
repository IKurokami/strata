// SPDX-License-Identifier: MIT

use std::os::windows::ffi::{OsStrExt, OsStringExt};

use gtk::{gio, prelude::*};

pub(super) const FILE_DROP_MIME: &str = "application/x.windows.CF_HDROP";
pub(super) const DROP_EFFECT_MIME: &str = "application/x.windows.Preferred DropEffect";

pub(super) struct ClipboardRead {
    clipboard: gtk::gdk::Clipboard,
    signal: Option<gtk::glib::SignalHandlerId>,
    changed: std::rc::Rc<std::cell::Cell<bool>>,
}

impl ClipboardRead {
    pub(super) fn new(clipboard: &gtk::gdk::Clipboard) -> Self {
        let changed = std::rc::Rc::new(std::cell::Cell::new(false));
        let notified = changed.clone();
        let signal = clipboard.connect_changed(move |_| notified.set(true));
        Self {
            clipboard: clipboard.clone(),
            signal: Some(signal),
            changed,
        }
    }
    pub(super) fn changed(&self) -> bool {
        self.changed.get()
    }
}

impl Drop for ClipboardRead {
    fn drop(&mut self) {
        if let Some(signal) = self.signal.take() {
            self.clipboard.disconnect(signal);
        }
    }
}

pub(super) fn file_provider(files: &[gio::File], cut: bool) -> gtk::gdk::ContentProvider {
    let files_provider =
        gtk::gdk::ContentProvider::for_value(&gtk::gdk::FileList::from_array(files).to_value());
    let Some(data) = file_drop_data(files) else {
        return files_provider;
    };
    gtk::gdk::ContentProvider::new_union(&[
        gtk::gdk::ContentProvider::for_bytes(FILE_DROP_MIME, &gtk::glib::Bytes::from_owned(data)),
        gtk::gdk::ContentProvider::for_bytes(
            DROP_EFFECT_MIME,
            &gtk::glib::Bytes::from_owned((if cut { 2u32 } else { 1u32 }).to_le_bytes().to_vec()),
        ),
        files_provider,
    ])
}

pub(super) async fn prefers_move(clipboard: &gtk::gdk::Clipboard) -> bool {
    let Ok((stream, _)) = clipboard
        .read_future(&[DROP_EFFECT_MIME], gtk::glib::Priority::DEFAULT)
        .await
    else {
        return false;
    };
    // Never read an unbounded registered clipboard payload.
    read_move_effect(&stream).await
}

pub(super) async fn read_files(clipboard: &gtk::gdk::Clipboard) -> Option<Vec<gio::File>> {
    if let Ok((stream, _)) = clipboard
        .read_future(&[FILE_DROP_MIME], gtk::glib::Priority::DEFAULT)
        .await
    {
        let mut data = Vec::new();
        loop {
            let chunk = stream
                .read_bytes_future(64 * 1024, gtk::glib::Priority::DEFAULT)
                .await
                .ok()?;
            if chunk.is_empty() {
                break;
            }
            if data.len() + chunk.len() > 8 * 1024 * 1024 {
                return None;
            }
            data.extend_from_slice(chunk.as_ref());
        }
        return files_from_drop_data(&data);
    }
    clipboard
        .read_value_future(
            gtk::gdk::FileList::static_type(),
            gtk::glib::Priority::DEFAULT,
        )
        .await
        .ok()?
        .get::<gtk::gdk::FileList>()
        .ok()
        .map(|list| list.files().to_vec())
}

fn files_from_drop_data(data: &[u8]) -> Option<Vec<gio::File>> {
    if data.len() < 24 {
        return None;
    }
    let offset = u32::from_le_bytes(data[..4].try_into().ok()?) as usize;
    let wide = u32::from_le_bytes(data[16..20].try_into().ok()?);
    if offset < 20 || !offset.is_multiple_of(2) || wide == 0 {
        return None;
    }
    let names = data.get(offset..)?;
    if !names.len().is_multiple_of(2) {
        return None;
    }
    let words = names
        .as_chunks::<2>()
        .0
        .iter()
        .map(|word| u16::from_le_bytes(*word))
        .collect::<Vec<_>>();
    let mut remaining = words.as_slice();
    let mut files = Vec::new();
    loop {
        let end = remaining.iter().position(|word| *word == 0)?;
        if end == 0 {
            return (!files.is_empty()).then_some(files);
        }
        if files.len() >= 4096 {
            return None;
        }
        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&remaining[..end]));
        if !path.is_absolute() {
            return None;
        }
        files.push(gio::File::for_path(path));
        remaining = &remaining[end + 1..];
    }
}

async fn read_move_effect(stream: &impl IsA<gio::InputStream>) -> bool {
    match stream
        .read_all_future([0u8; 4], gtk::glib::Priority::DEFAULT)
        .await
    {
        Ok((data, 4, None)) => data == 2u32.to_le_bytes(),
        Ok(_) => false,
        Err(_) => false,
    }
}

pub(super) fn file_drop_data(files: &[gio::File]) -> Option<Vec<u8>> {
    if files.is_empty() {
        return None;
    }
    // DROPFILES has five 32-bit fields on both Win32 and Win64. The trailing
    // names are absolute UTF-16 paths, individually terminated, then an empty name.
    let mut data: Vec<_> = [20_u32, 0, 0, 0, 1]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    for file in files {
        let path = file.path()?;
        if !path.is_absolute() {
            return None;
        }
        for unit in path.as_os_str().encode_wide() {
            if unit == 0 {
                return None;
            }
            data.extend_from_slice(&unit.to_le_bytes());
        }
        data.extend_from_slice(&0_u16.to_le_bytes());
    }
    data.extend_from_slice(&0_u16.to_le_bytes());
    Some(data)
}

#[cfg(test)]
mod tests;
