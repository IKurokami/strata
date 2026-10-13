//! Win32 metadata fingerprint without extra disk or permission probes.
//! The caller already has `std::fs::Metadata` from its existing background
//! read. No Shell API, ACL check, UAC or new syscall is performed here.
use std::{fs::Metadata, os::windows::fs::MetadataExt};

/// FILETIME creation/last-write values have 100ns resolution since 1601.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub creation_time: u64,
    pub last_write_time: u64,
    pub attributes: u32,
    pub size: u64,
}

pub fn from_metadata(metadata: &Metadata) -> FileStamp {
    FileStamp {
        creation_time: metadata.creation_time(),
        last_write_time: metadata.last_write_time(),
        attributes: metadata.file_attributes(),
        size: metadata.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn fingerprints_a_file_from_one_metadata_result() {
        let path = std::env::temp_dir().join(format!(
            "strata-file-stamp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, b"abc").expect("create fixture");
        let result = (|| {
            let metadata = std::fs::metadata(&path)?;
            let stamp = from_metadata(&metadata);
            assert_eq!(stamp.size, 3);
            assert!(stamp.creation_time > 0);
            assert!(stamp.last_write_time > 0);
            assert_eq!(stamp, from_metadata(&metadata));
            Ok::<(), std::io::Error>(())
        })();
        std::fs::remove_file(path).expect("remove fixture");
        result.expect("read fixture");
    }
}
