//! Windows-only, zero-I/O native path handling.
//!
//! No ACL checks, disk probes, shell extensions or permission prompts.
use std::{
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::{Component, Path, Prefix},
};

/// Differentiate absolute destinations from paths that require resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathKind {
    LocalAbsolute,
    NetworkAbsolute,
    DriveRelative,
    RootRelative,
    Relative,
    SpecialNamespace,
}

/// Pure lexical classification suitable for the synchronous UI entry handler.
pub fn classify(path: &Path) -> PathKind {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) => {
                if path.has_root() {
                    PathKind::LocalAbsolute
                } else {
                    PathKind::DriveRelative
                }
            }
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) => PathKind::NetworkAbsolute,
            Prefix::DeviceNS(_) | Prefix::Verbatim(_) => PathKind::SpecialNamespace,
        },
        Some(Component::RootDir) => PathKind::RootRelative,
        _ => PathKind::Relative,
    }
}

/// Determine whether volume identity lookups must be asynchronous.
/// A drive-letter path may be an offline mapped network drive even when it
/// looks local. This deliberately avoids any GetDriveType, GIO or ACL probe on
/// GTK's UI thread; it is not the same as classifying a path as network storage.
pub fn volume_query_may_block(path: &Path) -> bool {
    matches!(
        classify(path),
        PathKind::LocalAbsolute | PathKind::NetworkAbsolute
    )
}

/// Windows FILE_ATTRIBUTE_HIDDEN is provided with FindFirstFile/FindNextFile.
/// This is a bit test, not a metadata probe or a rule about dot-prefixed names.
pub const fn is_hidden_attributes(attributes: u32) -> bool {
    attributes & 0x0000_0002 != 0
}

/// Validate a single ordinary child component without touching the filesystem.
///
/// Avoid path traversal, alternate data streams, reserved Win32 device names
/// and names requiring verbatim Win32 semantics. No path normalization cost.
pub fn valid_child_name(name: &OsStr) -> bool {
    if !matches!(
        Path::new(name).components().next(),
        Some(Component::Normal(_))
    ) || Path::new(name).components().count() != 1
    {
        return false;
    }
    // A single UTF-16 pass, with no heap allocation per name. Reserved DOS
    // device stems never exceed seven UTF-16 code units (CONOUT$).
    let mut stem = [0u16; 7];
    let mut stem_len = 0usize;
    let mut meaningful_stem_len = 0usize;
    let mut in_stem = true;
    let mut last = None;
    for unit in name.encode_wide() {
        last = Some(unit);
        if unit < 32
            || matches!(
                unit,
                0x0022 // double quote
                    | 0x002a // star
                    | 0x002f // forward slash
                    | 0x003a // colon (alternate data stream)
                    | 0x003c // less-than
                    | 0x003e // greater-than
                    | 0x003f // question mark
                    | 0x005c // backslash
                    | 0x007c // pipe
            )
        {
            return false;
        }
        if in_stem {
            if unit == b'.' as u16 {
                in_stem = false;
            } else {
                if stem_len < stem.len() {
                    stem[stem_len] = match unit {
                        0x61..=0x7a => unit - 32, // ASCII uppercase
                        _ => unit,
                    };
                }
                stem_len = stem_len.saturating_add(1);
                if unit != 0x20 {
                    meaningful_stem_len = stem_len;
                }
            }
        }
    }
    if matches!(last, None | Some(0x20 | 0x2e)) {
        return false;
    }

    // Keep the last non-space position so arbitrarily many trailing spaces
    // cannot bypass reserved aliases such as "CON           .txt".
    let base = if meaningful_stem_len <= stem.len() {
        &stem[..meaningful_stem_len]
    } else {
        &stem[..0]
    };
    if matches!(
        base,
        [67, 79, 78] // CON
            | [80, 82, 78] // PRN
            | [65, 85, 88] // AUX
            | [78, 85, 76] // NUL
            | [67, 79, 78, 73, 78, 36] // CONIN$
            | [67, 79, 78, 79, 85, 84, 36] // CONOUT$
            | [67, 79, 77, 49..=57 | 0x00B9 | 0x00B2 | 0x00B3] // COM1-9 + superscript
            | [76, 80, 84, 49..=57 | 0x00B9 | 0x00B2 | 0x00B3] // LPT1-9 + superscript
    ) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_drive_unc_and_relative_without_disk_access() {
        assert_eq!(
            classify(Path::new(r"C:\Windows\System32")),
            PathKind::LocalAbsolute
        );
        assert_eq!(
            classify(Path::new(r"\\server\share\item")),
            PathKind::NetworkAbsolute
        );
        assert_eq!(classify(Path::new(r"C:relative")), PathKind::DriveRelative);
        assert_eq!(classify(Path::new(r"\relative")), PathKind::RootRelative);
        assert_eq!(classify(Path::new(r"relative\item")), PathKind::Relative);
    }

    #[test]
    fn unknown_mapped_drives_never_enter_sync_volume_probe() {
        for path in [
            Path::new(r"Z:\offline\folder"),
            Path::new(r"C:\Windows\System32"),
            Path::new(r"\\server\share\file"),
        ] {
            assert!(
                volume_query_may_block(path),
                "must defer volume lookup: {path:?}"
            );
        }
        for path in [Path::new(r"C:relative"), Path::new(r"relative\file")] {
            assert!(!volume_query_may_block(path));
        }
    }

    #[test]
    fn hidden_flag_is_not_inferred_from_unix_dot_names() {
        assert!(is_hidden_attributes(0x0000_0002));
        assert!(is_hidden_attributes(0x0000_0002 | 0x0000_0020));
        assert!(!is_hidden_attributes(0));
        assert!(!is_hidden_attributes(0x0000_0004));
    }

    #[test]
    fn reject_windows_filename_escapes_and_device_names() {
        for name in [
            "",
            ".",
            "..",
            r"a\b",
            "a/b",
            "C:relative",
            "evil:stream",
            "report.",
            "report ",
            "file?",
            "CON",
            "nul.txt",
            "COM1.log",
            "COM¹.txt",
            "LPT².txt",
            "CONIN$.ini",
            "CONOUT$",
            "CON .txt",
            "CON          .txt",
            "conout$.cfg",
            "COM3 .bak",
            "LPT9",
            "PRN",
            "AUX.ini",
            "file\u{0001}",
        ] {
            assert!(!valid_child_name(OsStr::new(name)), "must reject {name:?}");
        }
        for name in [
            "test",
            "a.txt",
            ".gitignore",
            "file with spaces.txt",
            "COnsole",
            "convenient.txt",
            "COM10.txt",
            "普通文件.txt",
        ] {
            assert!(valid_child_name(OsStr::new(name)), "should allow {name:?}");
        }
    }
}
