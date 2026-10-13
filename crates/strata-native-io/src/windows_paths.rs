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
    let units: Vec<u16> = name.encode_wide().collect();
    if units.is_empty() || matches!(units.last(), Some(0x20 | 0x2e)) {
        return false;
    }
    if units.iter().any(|&unit| {
        unit < 32
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
    }) {
        return false;
    }
    // Win32 resolves reserved device names even with file extensions.
    let base: Vec<u16> = units
        .iter()
        .copied()
        .take_while(|&unit| unit != b'.' as u16)
        .map(|unit| match unit {
            0x61..=0x7a => unit - 32, // ASCII uppercase
            _ => unit,
        })
        .collect();
    if matches!(
        base.as_slice(),
        [67, 79, 78] // CON
            | [80, 82, 78] // PRN
            | [65, 85, 88] // AUX
            | [78, 85, 76] // NUL
            | [67, 79, 77, 49..=57] // COM1 through COM9
            | [76, 80, 84, 49..=57] // LPT1 through LPT9
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
        ] {
            assert!(valid_child_name(OsStr::new(name)), "should allow {name:?}");
        }
    }
}
