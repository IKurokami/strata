# Windows parity audit — 2026-10-14

Scope: `IKurokami/strata` fork `main` at `5c192370add6b18cc70276ec27284a40e5c67a84`, cross-checked with `lgse/strata` Linux main. PR #24 (portable packaging and Linux CI fixes) is open and **not** part of the baseline. This is a source-level audit, **not** an assertion of Windows interactive GUI acceptance.

## Product rule

Preserve every user-facing Linux capability that can be ported or reimplemented with a native Windows API or safe cross-platform library. Omit only integrations that are *themselves* specific to Linux infrastructure (not the user-visible functionality they provide). Do not trade away security, responsiveness, or correct undo/trash semantics simply to remove an unsupported message.

## Outstanding portable parity work

| Priority | User-visible capability | Evidence in fork main | Windows completion gate |
| --- | --- | --- | --- |
| P0 | Rename, move, batch paste, overwrite/keep-both, merge, undo | `src/adapters/local_operations_windows.rs` rejects several operations; `crates/strata-native-io/src/windows_operations.rs` currently implements the bounded foundational worker | Race-safe native move/rename (including cross-volume semantics), no-clobber, cancellation, conflict UI and undo regression tests |
| P0 | Move to Recycle Bin / restore | `src/adapters/trash_restore_windows.rs` always refuses; Windows adapter does not substitute permanent delete | Shell recycle contract with verified item identity/original location, restore and undo tests |
| P0 | Image/PDF/document/video/audio/RAW previews, file thumbnails | `src/sandbox_windows.rs`, `src/sandbox/browser_windows.rs`, `src/sandbox/media_windows.rs`, `src/sandbox_helper_windows.rs` fail closed by design | Restricted decoder worker using Job Objects / Windows isolation and authenticated bounded IPC; then enable families incrementally |
| P1 | Browse/compress/extract archives | `src/adapters/local_operations_windows.rs` rejects native archive mutation and direct archive preview | Sandboxed codecs; destination traversal protection, atomic publish, cancellation |
| P1 | USB/removable drives, mapped drives, eject and volume facts | `src/adapters/volume/mounts_windows.rs` is a zero-I/O placeholder; removable flush explicitly unsupported | Background Win32 volume enumeration; mount identity, safe-eject and mapped-UNC detection without UI thread polling |
| P1 | External providers including remote integration | `src/services/file_providers_windows.rs` and `src/services/file_providers/transport_windows.rs` disable discovery and transport | ACL-validated provider ownership, named-pipe identity/authentication, contained workers; keep SFTP on-demand |
| P1 | Custom actions / external tasks | `src/adapters/local_jobs_windows.rs` rejects native execution | Scoped process tree cancellation and resource limits using Windows Job Objects; no PowerShell command-string interpolation |
| P1 | In-app automatic install/update | `src/services/update_install_windows.rs` refuses install/update | Signed verified updater with safe process exit/restart and rollback; keep release checks |
| P2 | Native thumbnail cache | `src/ui/thumbnail_cache_windows.rs` is intentionally unavailable | Secure bounded cache and invalidation using stable Win32 file IDs |
| P2 | Production-quality Windows packaging and CI coverage | `.github/workflows/windows-gui-linked-smoke.yml` and `scripts/windows/package-portable.ps1` cover experimental packaging; PR #24 is pending | Clean Windows runner GUI first paint, folder/preview E2E, signed installer, full test matrix, first-use performance |

The cross-platform GTK interface, tabs, search, folder listing, creation and basic copying already have code paths on Windows, but **their actual parity is not established by a successful compile or `--version` smoke**. Measure and test the interactive application before marking them complete.

## Windows-only integrations added in this branch

1. **Open in Windows Terminal** (`wt.exe -d PATH`): existing folder's "Open in Terminal" action now prefers the native Windows Terminal. Fallbacks are PowerShell 7 (`pwsh.exe`), Windows PowerShell (`powershell.exe`) and Command Prompt (`cmd.exe`), inheriting the folder as process working directory. This is a lazy on-click operation, with no PowerShell command text interpolation, privilege escalation or background scanner.
2. **Native File Explorer context-menu actions**: "Open in File Explorer" for the current local directory and "Show in File Explorer" for one selected local item. Explorer is launched from `SystemRoot\explorer.exe`, not by searching an untrusted working directory or PATH. Trash and remote locations are excluded.
3. **Windows clipboard paths**: "Copy path" / "Copy paths" preserve Windows backslash paths, spaces, UNC syntax and punctuation as native text instead of applying Linux shell escapes.

These changes are optional integrations; Linux behavior is left unchanged with `#[cfg(windows)]` boundaries.

## Deliberately *not* porting Linux-specific infrastructure

D-Bus FileManager1, XDG portal plumbing, GVfs command health checks, UDisks2/udiskie, Omarchy, X11/Wayland-specific launch hooks, pacman/AUR, Bubblewrap, Landlock and seccomp are Linux infrastructure rather than Windows product features. They must remain intact on Linux. Their **equivalent user capabilities** (file dialog, drive handling, task containment, updater) remain in the parity table where necessary; do not delete those features from Windows.

## Future Windows-exclusive ideas (not implemented by this PR)

- Optional Explorer/Quick Access integration and Windows 11 "Open with Strata" registration, explicitly opt-in rather than touching the registry at startup.
- Optional NTFS/USN-journal fast search, volume-aware with an ordinary filesystem-walk fallback and no administrator requirement for everyday browsing.
- Windows Shell property inspection / file attributes and verified Windows shortcuts (`.lnk`) with safe background resolution.
- Optional WSL path bridging (`\\wsl$\...`) with graceful offline handling.
- Per-monitor DPI, Windows accent/theme, jump lists and taskbar progress; add only where measurable value exceeds process startup overhead.

## Acceptance criteria

1. Windows native I/O, linked GTK build, new native terminal/Explorer/clipboard unit tests, Linux formatting/lints/tests and E2E must pass.
2. On real Windows, check Explorer selection for names with spaces and metacharacters, Windows Terminal working directory, fallback to PowerShell/cmd when Terminal is absent, long local paths and UNC/offline paths.
3. Verify no background process/probe, UAC prompt, untrusted shell parsing, synchronous directory metadata enumeration or Linux feature regression.
4. Rebase after PR #24 is merged, then rerun full CI; do not merge this branch based only on `cargo check`.

## Upstream drift verified against `lgse/strata`

GitHub's cross-fork commit comparison reports **8 upstream commits** beyond this fork's `main` as of the audit. This is commit ancestry, **not automatically eight missing features**: upstream `d0f7028` (compact sidebar rail) was manually reapplied to the fork in PR #21, and upstream `5e159343` (copy-cancellation cleanup) overlaps the fix staged in open PR #24.

Remaining upstream patches to compare and reapply carefully (without resetting fork commits):

- `c0490319`, `586ca3c9`: Simplified Chinese localization fixes.
- `59c67222`: archive extraction conflict handling for dotted folder names.
- `6aa721f7`: preview-type detection using file bytes rather than extensions.
- `58981b33`: PDF preview zoom decoupled from interface font size.
- `c85a001d`: horizontal scrolling for unwrapped rendered documents.

Do not label preview behavior ported merely by copying these patches: the Windows secure decoder worker remains unavailable. Track this upstream drift separately from the Windows-native implementation backlog; reapply each patch with conflict review, test coverage and intact Linux behavior.
