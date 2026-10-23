// SPDX-License-Identifier: MIT

use super::*;
use std::os::windows::ffi::OsStringExt;
use std::time::{Duration, Instant};
use windows::{
    Win32::{Foundation::PROPERTYKEY, UI::Shell::IShellItem2},
    core::{GUID, Interface},
};

/// A Shell-verified item; the physical identity distinguishes repeated deletions
/// from the same original path.
#[derive(Clone, Debug)]
pub struct RecycleBinEntry {
    pub item: RecycledItem,
}

pub struct RecycleBinListing {
    pub entries: Vec<RecycleBinEntry>,
    pub truncated: bool,
}

fn decode_original_record(bytes: &[u8]) -> io::Result<PathBuf> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid Recycle Bin metadata");
    if bytes.len() < 28 || bytes.len() > 65_536 {
        return Err(invalid());
    }
    let version = u64::from_le_bytes(bytes[..8].try_into().map_err(|_| invalid())?);
    let name = match version {
        1 if bytes.len() == 544 => &bytes[24..],
        2 => {
            let words =
                u32::from_le_bytes(bytes[24..28].try_into().map_err(|_| invalid())?) as usize;
            let end = 28usize
                .checked_add(words.checked_mul(2).ok_or_else(invalid)?)
                .ok_or_else(invalid)?;
            if end != bytes.len() {
                return Err(invalid());
            }
            &bytes[28..end]
        }
        _ => return Err(invalid()),
    };
    let words = name
        .chunks_exact(2)
        .map(|word| u16::from_le_bytes([word[0], word[1]]))
        .collect::<Vec<_>>();
    let end = words
        .iter()
        .position(|word| *word == 0)
        .ok_or_else(invalid)?;
    if words[end..].iter().any(|word| *word != 0) {
        return Err(invalid());
    }
    let path = PathBuf::from(std::ffi::OsString::from_wide(&words[..end]));
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(invalid());
    }
    Ok(path)
}

fn children() -> io::Result<IEnumShellItems> {
    // SAFETY: called only inside the initialized STA; returned interfaces stay there.
    let bin: IShellItem =
        unsafe { SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None) }
            .map_err(|error| io_error("cannot open the Recycle Bin", &error))?;
    // SAFETY: the live known-folder item supports Shell item enumeration.
    unsafe { bin.BindToHandler(None, &BHID_EnumItems) }
        .map_err(|error| io_error("cannot list the Recycle Bin", &error))
}

fn next(children: &IEnumShellItems) -> io::Result<Option<IShellItem>> {
    let mut batch = [None];
    let mut fetched = 0;
    // SAFETY: both output buffers remain valid throughout the call.
    unsafe { children.Next(&mut batch, Some(&mut fetched)) }
        .map_err(|error| io_error("cannot enumerate the Recycle Bin", &error))?;
    Ok(if fetched == 0 { None } else { batch[0].take() })
}

pub(super) fn original(item: &IShellItem) -> io::Result<PathBuf> {
    let item: IShellItem2 = item
        .cast()
        .map_err(|error| io_error("cannot read deleted item properties", &error))?;
    // Compare the Shell's original directory with the companion record;
    // display-name properties may omit known extensions.
    let key = PROPERTYKEY {
        fmtid: GUID::from_u128(0x9b174b33_40ff_11d2_a27e_00c04fc30871),
        pid: 2,
    };
    // SAFETY: the property key and live Shell item remain valid; free the Shell allocation below.
    let value = unsafe { item.GetString(&key) }
        .map_err(|error| io_error("cannot read the original location", &error))?;
    if value.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing original location",
        ));
    }
    // SAFETY: a successful GetString returns a task-allocated NUL-terminated string.
    let path = PathBuf::from(std::ffi::OsString::from_wide(unsafe { value.as_wide() }));
    // SAFETY: matches the task allocator used by GetString.
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    // Display names omit known extensions according to Explorer preferences.
    // Read the bounded companion record only after Shell supplied this identity.
    let recycled = item_path(&item)?;
    let mut info_name = recycled
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing bin filename"))?
        .encode_wide()
        .collect::<Vec<_>>();
    if !info_name.starts_with(&[b'$' as u16, b'R' as u16]) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid bin identity",
        ));
    }
    info_name[1] = b'I' as u16;
    let info_path = recycled.with_file_name(std::ffi::OsString::from_wide(&info_name));
    use std::io::Read;
    let mut bytes = Vec::new();
    File::open(info_path)?
        .take(65_537)
        .read_to_end(&mut bytes)?;
    let original = decode_original_record(&bytes)?;
    if original.parent() != Some(path.as_path()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "original location disagrees with Shell",
        ));
    }
    let path = original;
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid original location",
        ));
    }
    Ok(path)
}

/// Enumerate the current user's Shell namespace, across volumes, within a budget.
pub fn list_recycled(max_entries: usize, time_budget: Duration) -> io::Result<RecycleBinListing> {
    with_sta(move || {
        let deadline = Instant::now().checked_add(time_budget);
        let children = children()?;
        let mut entries = Vec::new();
        let mut unreadable = false;
        let mut visited = 0usize;
        loop {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(RecycleBinListing {
                    entries,
                    truncated: true,
                });
            }
            let Some(child) = next(&children)? else { break };
            if visited >= max_entries {
                return Ok(RecycleBinListing {
                    entries,
                    truncated: true,
                });
            }
            visited += 1;
            match original(&child).and_then(|original| {
                Ok(RecycledItem {
                    original,
                    recycled: item_path(&child)?,
                })
            }) {
                Ok(item) => entries.push(RecycleBinEntry { item }),
                Err(_) => unreadable = true,
            }
        }
        Ok(RecycleBinListing {
            entries,
            truncated: unreadable,
        })
    })
}

#[cfg(test)]
mod tests;

/// Resolve only an identity actually present in the current user's Shell bin.
pub fn original_path(recycled: &Path) -> io::Result<PathBuf> {
    let recycled = recycled.to_path_buf();
    with_sta(move || original(&find_in_bin(&recycled)?))
}

/// Permanently delete a verified Shell bin item, preserving its metadata bookkeeping.
pub fn delete_recycled(recycled: &Path) -> io::Result<()> {
    let recycled = recycled.to_path_buf();
    with_sta(move || {
        let source = find_in_bin(&recycled)?;
        let outcomes = Rc::new(RefCell::new(Outcomes::default()));
        let sink: IFileOperationProgressSink = Sink {
            outcomes: outcomes.clone(),
            refuse_existing_destination: false,
            expect_recycled_item: false,
        }
        .into();
        let operation = new_operation(FOF_NOERRORUI | FOF_NOCONFIRMATION)?;
        // SAFETY: the source is a verified bin item and both interfaces live through perform.
        unsafe { operation.DeleteItem(&source, &sink) }
            .map_err(|error| io_error("cannot queue bin deletion", &error))?;
        perform(&operation, &outcomes)
    })
}
