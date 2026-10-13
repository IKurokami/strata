# Strata Windows feature compatibility

The Windows build is still a work in progress. A green `strata-native-io` CI job only proves the **portable/native I/O crate** builds and passes tests, not that the GTK application starts on Windows.

## Porting decision rules

1. Keep the same visible interaction when it can be implemented using Windows-native APIs and bounded background work without elevated permissions.
2. If an integration is purely Linux desktop infrastructure, omit **only the Windows control and startup probe**; keep Linux behavior unchanged.
3. Do not silently replace *trash* with permanent deletion, executable sandboxing with unsandboxed parsing, or cross-volume move with an expensive synchronous copy.
4. Prefer fast lazy results: names first; file type from enumeration metadata if available; thumbnails, signatures, ACLs, Shell extensions and network shares only on demand. No synchronous UAC or admin prompt on ordinary browsing.
5. Do not add a mandatory NTFS MFT/USN index: Everything-like indexing may be optional but needs permission/volume-aware fallback.

## Feature decisions

| Feature | Windows plan | State |
| --- | --- | --- |
| GTK4 views, keyboard actions, tabs, appearance | Retain UI; use GDK Win32 backend | Windows GUI unverified |
| Listing local folders | Use lazy streaming batches and Windows-provided type hints | Backend tested, GTK integration pending |
| Filename search | Preserve Rust matching/scoring; scan without startup index; optional opt-in search index later | Main-app integration pending |
| Create folder / copy file | Preserve with background, bounded queue and exclusive destination creation | Basic native worker implemented; GTK integration pending |
| Rename / move | Keep functionality using safe native rename semantics, with prompt on collision/cross-volume move | Not yet implemented |
| Recycle Bin | Port to Windows shell recycle semantics | Not yet implemented; **never** fall back to permanent delete |
| USB, disks, volumes | Use Windows enumeration; no UDisks2 requirement | Not yet implemented |
| SMB and network paths | Prefer native UNC; avoid blocking initial local listing on network volumes | Path classification implemented, remote operations unverified |
| SFTP | Optional network backend, on-demand only | Not yet implemented |
| Preview / PDF / video / RAW | Retain, with Windows-compatible hardened helper isolation before enabling untrusted parsers | Raw-byte prefetch exists; sandbox not ported |
| Open With | Keep GIO AppInfo associations; avoid `gio_unix::DesktopAppInfo` on Windows | Conditional Unix implementation removed; GUI unverified |
| Show hidden files | Keep Windows hidden attribute behavior; do not read GNOME `.hidden` | Windows skips Linux-only probe |
| D-Bus / XDG portals / FileManager1 | Not applicable on Windows; use Windows shell integration separately | Linux-only Settings controls excluded from Windows |
| Omarchy keybinds and `udiskie` | Linux-specific; do not ship on Windows | Windows settings excluded |
| BitLocker | Leave to Windows, do not request elevation merely for file listing | Native integration pending |
| Startup/update/release | Native no-probe startup; Windows installer and separate updater | Not yet implemented |

## CI and blockers

- Independent crate CI runs formatting, Clippy, tests and release compilation on Windows Server 2025 and Ubuntu.
- The upstream full-app E2E workflow depends on Linux image publishing to GHCR. For the fork, a future published fork image can be reused; when unavailable, `ci.yml` now explicitly builds an ephemeral test image rather than skipping validation.
- Full GTK `strata.exe`, Windows previews, sandbox isolation, File Explorer integration, and first-frame benchmarks are still required before release.
