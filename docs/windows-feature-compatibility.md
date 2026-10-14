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
| Create folder / copy file | Preserve with background, bounded queue and exclusive destination creation | Recursive directory copy now uses the native worker; reparse points are rejected and existing destinations are preserved |
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
| Startup/update/release | Skip Linux GVfs health checks, portal refresh, X11 and D-Bus launch hooks on Windows; implement native updater and installer separately | Linux startup probes now target-gated; Windows app and installer not yet built |

## Phase 3: platform-specific fast paths

- Recursive filename search skips Unix mode bit checks on Windows rather than issuing one metadata request per file. It retains the bounded walker and index behavior.
- Icon details cache timestamps use Windows FILETIME / basic file attributes; this does not trigger any Shell/COM permission check. Cache refresh is still performed on the application side and must be profiled for UI-thread blocking after GTK integration.
- Ordinary Windows PE executables are recognized from `.exe` / `.com` extensions without checking Unix permissions. Batch scripts are not launched implicitly.
- Enumeration exposes `hidden_hint` from `DirEntry::metadata` on Windows, where this data is supplied by the directory enumeration result; Linux leaves this hint unset.
- Windows native folder inode-identity recovery is temporarily disabled because portable stable Rust does not provide a reliable Windows per-file identity. Rename notification recovery needs a dedicated Windows file-ID implementation; never infer identity solely from filenames or creation timestamps.
- The complete GTK GUI is still not compiled or tested on Windows. No end-to-end performance figures are claimed.

## Phase 5: Windows entry-point portability

- The `--gvfs-probe` dispatch and Unix `udiskie` hook execution are compiled only for Linux; Windows must not compile Unix process `exec` or register GVfs-only launch behavior.
- The `gdk4_x11::X11Display` helper is now Linux-only, matching the target-gated `gdk4-x11` dependency. Windows startup does not load X11 or run the Linux GVfs probe.
- These guards remove *specific* Windows compilation blockers without changing Linux entry points. `portal_setup`, sandboxing, native volume support and the rest of the GTK application still need platform separation; this is not proof of a Windows GUI build.

## Phase 6: Windows volume and mount snapshots

- Linux retains `/proc/self/mountinfo` and `gio_unix::MountMonitor`; Windows now selects separate zero-I/O adapters so no Unix mount parsing or GIO mount-monitor setup occurs during ordinary Windows navigation.
- Explicit UNC roots are recognized lexically as potentially blocking network paths. Remote volume queries are still delegated to the asynchronous path, not the hot UI thread.
- Unknown drive types, mapped network letters, removable volumes and their stable identities are **not** guessed. The Windows snapshot intentionally has no removable entries until a real asynchronous Win32 enumerator exists; an empty snapshot must not be mistaken for proof that no removable disks exist.
- Windows Recycle Bin remains disabled rather than using Linux's trash mount scan or silently substituting permanent deletion. Native volume APIs and mapped network drive classification remain necessary before feature parity.

## CI and blockers

- Independent crate CI runs formatting, Clippy, tests and release compilation on Windows Server 2025 and Ubuntu.
- The upstream full-app E2E workflow depends on Linux image publishing to GHCR. For the fork, a future published fork image can be reused; when unavailable, `ci.yml` now explicitly builds an ephemeral test image rather than skipping validation.
- Full GTK `strata.exe`, Windows previews, sandbox isolation, File Explorer integration, and first-frame benchmarks are still required before release.

## Performance decisions implemented in phase 2

- Startup: skip the Linux GVfs subprocess probe and XDG portal maintenance on Windows. Do **not** trigger those 2-second Linux health-check paths from a Win32 launch.
- Settings: omit Linux-only portal and udiskie integration controls on Windows rather than probing unavailable services.
- Native local operations: queue `CreateDirectory` and no-clobber `CopyFile` off the UI thread with a bounded FIFO; no privilege elevation or permanent-delete fallback.
- Open With: rely on platform GIO AppInfo; no `gio_unix::DesktopAppInfo` lookup on Windows.
- Full application integration is still pending, so none of these changes establish measured Windows startup/preview improvement yet.
