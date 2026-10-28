// SPDX-License-Identifier: MIT

/// Version-one, tightly packed, unpremultiplied RGBA8. Legacy PNG remains valid
/// on the same length-bounded worker transport.
pub(crate) fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(..8)? != b"STRGBA01" {
        return None;
    }
    let width = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?);
    let height = u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?);
    if width == 0 || height == 0 || width > 2560 || height > 2560 {
        return None;
    }
    let length = u64::from(width)
        .checked_mul(u64::from(height))?
        .checked_mul(4)?
        .checked_add(16)?;
    (length == bytes.len() as u64 && length <= 32 * 1024 * 1024).then_some((width, height))
}

pub(crate) fn decode(mut bytes: Vec<u8>) -> Result<((u32, u32), Vec<u8>), String> {
    let size = dimensions(&bytes).ok_or("Invalid sandbox RGBA payload")?;
    bytes.drain(..16);
    Ok((size, bytes))
}

#[cfg(test)]
#[path = "pixels/tests.rs"]
mod tests;
use std::sync::atomic::{AtomicBool, Ordering};

static FAST_RAW: AtomicBool = AtomicBool::new(true);
pub(crate) fn set_fast_raw(enabled: bool) {
    FAST_RAW.store(enabled, Ordering::Release);
}
pub(crate) fn fast_raw() -> bool {
    FAST_RAW.load(Ordering::Acquire)
}
