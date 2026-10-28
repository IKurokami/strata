// SPDX-License-Identifier: MIT

use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) const REVISION: &str = "bfcd298762cc34d0357ece5ebdd31791a3a374d8";
pub(super) const FINGERPRINT: &str = "embeddinggemma2:bfcd298762cc34d0357ece5ebdd31791a3a374d8:q8_0:llama-vectors1:768:strata-segments1:280";
pub(super) const TOTAL_BYTES: u64 = 864_676_480;
const MODELS: [(&str, u64, &str); 2] = [
    (
        "embeddinggemma-2-Q8_0.gguf",
        309_855_456,
        "2188ac1deca4b77dffefd603c2776a9d76d9d74ec01841392982ebb840b09135",
    ),
    (
        "mmproj-embeddinggemma-2-Q8_0.gguf",
        554_821_024,
        "c4a8a52691ecef40618438928bdf9e68379b854e24166f292592353db0aab64f",
    ),
];

pub(super) fn folder(data: &Path) -> PathBuf {
    data.join("models").join(REVISION)
}
pub(super) fn ready(data: &Path) -> bool {
    let folder = folder(data);
    fs::symlink_metadata(folder.join("verified.json")).is_ok_and(|metadata| {
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() == FINGERPRINT.len() as u64
    }) && fs::read_to_string(folder.join("verified.json")).is_ok_and(|value| value == FINGERPRINT)
        && MODELS.iter().all(|(name, size, _)| {
            fs::symlink_metadata(folder.join(name)).is_ok_and(|metadata| {
                metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() == *size
            })
        })
}

pub(super) fn download(
    data: &Path,
    cancel: &AtomicBool,
    progress: impl Fn(u64),
) -> Result<(), String> {
    let folder = folder(data);
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    if fs::symlink_metadata(&folder)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Model directory must not be a symbolic link".into());
    }
    let mut completed = 0;
    for (name, size, digest) in MODELS {
        let destination = folder.join(name);
        if regular(&destination)? && valid(&destination, size, digest, cancel)? {
            completed += size;
            progress(completed);
            continue;
        }
        let partial = folder.join(format!("{name}.partial"));
        let exists = regular(&partial)?;
        let mut offset = if exists {
            fs::metadata(&partial)
                .map_err(|error| error.to_string())?
                .len()
        } else {
            0
        };
        if offset == size && valid(&partial, size, digest, cancel)? {
            if regular(&destination)? {
                fs::remove_file(&destination).map_err(|error| error.to_string())?;
            }
            fs::rename(&partial, &destination).map_err(|error| error.to_string())?;
            completed += size;
            progress(completed);
            continue;
        }
        if offset >= size {
            fs::remove_file(&partial).map_err(|error| error.to_string())?;
            offset = 0;
        }
        let url = format!(
            "https://huggingface.co/ggml-org/embeddinggemma-2-GGUF/resolve/{REVISION}/{name}"
        );
        if cancel.load(Ordering::Acquire) {
            return Err("Model download cancelled".into());
        }
        let mut request = ureq::get(&url);
        if offset > 0 {
            request = request.header("Range", &format!("bytes={offset}-"));
        }
        let mut response = request.call().map_err(|error| error.to_string())?;
        let resume = response.status().as_u16() == 206 && offset > 0;
        if resume {
            let expected = format!("bytes {offset}-");
            if !response
                .headers()
                .get("content-range")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| {
                    value.starts_with(&expected) && value.ends_with(&format!("/{size}"))
                })
            {
                return Err("Invalid model download range".into());
            }
        } else {
            offset = 0;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(!resume)
            .append(resume)
            .open(&partial)
            .map_err(|error| error.to_string())?;
        let mut source = response.body_mut().as_reader();
        let mut buffer = vec![0; 128 * 1024];
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Model download cancelled".into());
            }
            let count = source
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            offset += count as u64;
            if offset > size {
                return Err("Model download exceeds its pinned size".into());
            }
            output
                .write_all(&buffer[..count])
                .map_err(|error| error.to_string())?;
            progress(completed + offset);
        }
        output.sync_all().map_err(|error| error.to_string())?;
        drop(output);
        if !valid(&partial, size, digest, cancel)? {
            return Err("Model checksum mismatch".into());
        }
        if regular(&destination)? {
            fs::remove_file(&destination).map_err(|error| error.to_string())?;
        }
        fs::rename(&partial, &destination).map_err(|error| error.to_string())?;
        completed += size;
    }
    let marker = folder.join("verified.json.partial");
    regular(&marker)?;
    fs::write(&marker, FINGERPRINT).map_err(|error| error.to_string())?;
    fs::rename(marker, folder.join("verified.json")).map_err(|error| error.to_string())
}

fn regular(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err("Model paths must be regular files".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn valid(path: &Path, size: u64, expected: &str, cancel: &AtomicBool) -> Result<bool, String> {
    let mut source = fs::File::open(path).map_err(|error| error.to_string())?;
    if source.metadata().map_err(|error| error.to_string())?.len() != size {
        return Ok(false);
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err("Model verification cancelled".into());
        }
        let count = source
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()) == expected)
}

#[cfg(test)]
mod tests;

pub(super) fn verify(data: &Path, cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if !ready(data) {
        return Err("Smart Search models are not installed".into());
    }
    for (name, size, expected) in MODELS {
        let mut source =
            fs::File::open(folder(data).join(name)).map_err(|error| error.to_string())?;
        if source.metadata().map_err(|error| error.to_string())?.len() != size {
            return Err("Model size changed".into());
        }
        let mut digest = Sha256::new();
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            if cancelled() {
                return Err("Model verification cancelled".into());
            }
            let count = source
                .read(&mut buffer)
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        if hex::encode(digest.finalize()) != expected {
            return Err("Model checksum mismatch".into());
        }
    }
    Ok(())
}
