// SPDX-License-Identifier: MIT

use std::{
    io::{self, Read},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static RUNNING: AtomicBool = AtomicBool::new(false);

pub(crate) struct ComparisonPermit {
    _private: (),
}

impl ComparisonPermit {
    pub(crate) fn reserve() -> Option<Self> {
        RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self { _private: () })
    }
}

impl Drop for ComparisonPermit {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_LINES: usize = 10_000;
const MAX_CELLS: usize = 2_000_000;
const MAX_OUTPUT: usize = 256 * 1024;

#[derive(Debug)]
pub(crate) enum CompareError {
    Unsupported,
    TooLarge,
    TooComplex,
    Cancelled,
    Io(io::Error),
}

fn read_text(path: &Path, cancelled: &AtomicBool) -> Result<String, CompareError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(CompareError::Cancelled);
    }
    #[cfg(windows)]
    if !path.is_absolute() || !strata_win_shell::is_fixed_local_drive(path) {
        return Err(CompareError::Unsupported);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(CompareError::Io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CompareError::Unsupported);
    }
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        if metadata.file_attributes() & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0 {
            return Err(CompareError::Unsupported);
        }
        // Refuse leaf reparse points/cloud recall, and concurrent writers, while allowing deletion.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x4)
            .custom_flags(0x0020_0000 | 0x0010_0000)
            .open(path)
            .map_err(CompareError::Io)?;
        if file.metadata().map_err(CompareError::Io)?.file_attributes() & 0x400 != 0 {
            return Err(CompareError::Unsupported);
        }
        file
    };
    #[cfg(not(windows))]
    let file = std::fs::File::open(path).map_err(CompareError::Io)?;
    let before = file.metadata().map_err(CompareError::Io)?;
    if !before.is_file() {
        return Err(CompareError::Unsupported);
    }
    if before.len() > MAX_BYTES {
        return Err(CompareError::TooLarge);
    }
    let mut bytes = Vec::new();
    let mut input = (&file).take(MAX_BYTES + 1);
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(CompareError::Cancelled);
        }
        let count = input.read(&mut chunk).map_err(CompareError::Io)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let after = file.metadata().map_err(CompareError::Io)?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(CompareError::Unsupported);
    }
    if bytes.len() as u64 > MAX_BYTES {
        return Err(CompareError::TooLarge);
    }
    if bytes.contains(&0) {
        return Err(CompareError::Unsupported);
    }
    String::from_utf8(bytes).map_err(|_| CompareError::Unsupported)
}

pub(crate) fn compare(
    _permit: &ComparisonPermit,
    left: &Path,
    right: &Path,
    cancelled: &AtomicBool,
) -> Result<String, CompareError> {
    let left = read_text(left, cancelled)?;
    if cancelled.load(Ordering::Acquire) {
        return Err(CompareError::Cancelled);
    }
    let right = read_text(right, cancelled)?;
    diff(&left, &right, cancelled)
}

pub(crate) fn diff(
    left: &str,
    right: &str,
    cancelled: &AtomicBool,
) -> Result<String, CompareError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(CompareError::Cancelled);
    }
    if left.len().max(right.len()) as u64 > MAX_BYTES {
        return Err(CompareError::TooLarge);
    }
    let start = Instant::now();
    let left = left.replace("\r\n", "\n");
    let right = right.replace("\r\n", "\n");
    let a: Vec<_> = left.split_inclusive('\n').collect();
    let b: Vec<_> = right.split_inclusive('\n').collect();
    if a.len().max(b.len()) > MAX_LINES {
        return Err(CompareError::TooLarge);
    }
    let prefix = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    if prefix == a.len() && prefix == b.len() {
        return Ok(String::new());
    }
    let x = &a[prefix..a.len() - suffix];
    let y = &b[prefix..b.len() - suffix];
    let width = y.len() + 1;
    let cells = (x.len() + 1)
        .checked_mul(width)
        .filter(|cells| *cells <= MAX_CELLS)
        .ok_or(CompareError::TooComplex)?;
    let mut lengths = vec![0_u16; cells];
    for i in (0..x.len()).rev() {
        if cancelled.load(Ordering::Acquire) {
            return Err(CompareError::Cancelled);
        }
        if start.elapsed() > Duration::from_secs(2) {
            return Err(CompareError::TooComplex);
        }
        for j in (0..y.len()).rev() {
            lengths[i * width + j] = if x[i] == y[j] {
                lengths[(i + 1) * width + j + 1] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    let mut output = String::new();
    let from = prefix.saturating_sub(3);
    let old_count = prefix - from + x.len() + suffix.min(3);
    let new_count = prefix - from + y.len() + suffix.min(3);
    output.push_str(&format!(
        "@@ -{},{old_count} +{},{new_count} @@\n",
        from + usize::from(old_count > 0),
        from + usize::from(new_count > 0)
    ));
    for line in &a[from..prefix] {
        append_line(&mut output, ' ', line);
    }
    let (mut i, mut j) = (0, 0);
    while i < x.len() || j < y.len() {
        if cancelled.load(Ordering::Acquire) {
            return Err(CompareError::Cancelled);
        }
        if i < x.len() && j < y.len() && x[i] == y[j] {
            append_line(&mut output, ' ', x[i]);
            i += 1;
            j += 1;
        } else if i < x.len()
            && (j == y.len() || lengths[(i + 1) * width + j] >= lengths[i * width + j + 1])
        {
            append_line(&mut output, '-', x[i]);
            i += 1;
        } else {
            append_line(&mut output, '+', y[j]);
            j += 1;
        }
        if output.len() > MAX_OUTPUT {
            return Err(CompareError::TooComplex);
        }
    }
    for line in a[a.len() - suffix..].iter().take(3) {
        append_line(&mut output, ' ', line);
    }
    if output.len() > MAX_OUTPUT {
        return Err(CompareError::TooComplex);
    }
    Ok(output)
}

fn append_line(output: &mut String, prefix: char, line: &str) {
    output.push(prefix);
    output.push_str(line.trim_end_matches('\n'));
    output.push('\n');
    if !line.ends_with('\n') {
        output.push_str("\\ No newline at end of file\n");
    }
}

#[cfg(test)]
mod tests;
