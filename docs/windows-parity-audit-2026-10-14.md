# Windows parity audit — 2026-10-14

Scope: `IKurokami/strata` fork, cross-checked with `lgse/strata` Linux main. PR #25 now contains the complete PR #24 head `5feda688` as a real second-parent merge (`ac599652`), including portable packaging, the Linux copy-cancellation fix and the E2E scroll fix. This is a source-level audit, **not** an assertion of Windows interactive GUI acceptance.

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
| P2 | Production-quality Windows packaging and CI coverage | `.github/workflows/windows-gui-linked-smoke.yml` and `scripts/windows/package-portable.ps1` cover experimental packaging; PR #24 was imported into PR #25; final merge and acceptance remain pending | Clean Windows runner GUI first paint, folder/preview E2E, signed installer, full test matrix, first-use performance |

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
4. Rerun and pass full CI on the combined PR #25 head. When PR #24 lands on `main`, verify no duplicate or conflicting integration before merging #25; do not merge based only on `cargo check`.

## Upstream drift verified against `lgse/strata`

GitHub's cross-fork commit comparison reports **8 upstream commits** beyond this fork's `main` as of the audit. This is commit ancestry, **not automatically eight missing features**: upstream `d0f7028` (compact sidebar rail) was manually reapplied to the fork in PR #21, and upstream `5e159343` (copy-cancellation cleanup) overlaps the fix staged in open PR #24.

Remaining upstream patches to compare and reapply carefully (without resetting fork commits):

- `c0490319`, `586ca3c9`: Simplified Chinese localization fixes.
- `59c67222`: archive extraction conflict handling for dotted folder names.
- `6aa721f7`: preview-type detection using file bytes rather than extensions.
- `58981b33`: PDF preview zoom decoupled from interface font size.
- `c85a001d`: horizontal scrolling for unwrapped rendered documents.

Do not label preview behavior ported merely by copying these patches: the Windows secure decoder worker remains unavailable. Track this upstream drift separately from the Windows-native implementation backlog; reapply each patch with conflict review, test coverage and intact Linux behavior.

## Rust fast-delete source review (suptejas/delete, suptejas/turbo-delete)

The upstream **source**, not marketing throughput figures, was reviewed on 2026-10-14:

- [suptejas/delete](https://github.com/suptejas/delete/blob/master/src/lib.rs) uses `jwalk` to collect **all paths into two growing vectors** and creates futures for fixed-size chunks; each chunk iterates deletions sequentially. File deletion `unwrap()` can panic; directory removal errors and final `remove_dir_all` errors are ignored while the method reports `Ok(())`. It assumes UTF-8 filenames (`to_str().unwrap()`) and traverses with `unwrap()`. This is not a cancellation-safe Windows file-manager backend.
- [suptejas/turbo-delete](https://github.com/suptejas/turbo-delete/blob/master/src/main.rs) uses `jwalk`, `rayon` and a pool, but sets `follow_links(true)`; users can select directories with symlinks/reparse points/junctions. It collects every directory into memory before deleting. It recursively removes directories in parallel and tries again on leftovers, suppressing intermediate errors. A fallback clears read-only flags on entire contents, changing original permissions. These semantics are unsuitable for Strata's trust model and undo rules.
- **Decision:** neither library is imported as a dependency, and neither may replace the Recycle Bin or irreversible-delete path without a separate correctness gate. Explore the algorithmic **idea** of parallel discovery + bounded workers, but preserve explicit errors, cancellation, final-component and parent reparse-point policy, reliable progress, no unintended link traversal and recoverability. The reported 2–3× library speedup is not a Strata measurement.

### Windows-native performance candidates to benchmark before implementation

1. **Perceived latency:** keep the UI menu responsive immediately; once an explicitly confirmed delete is requested, send progress/cancellation through a bounded off-main-thread queue. Track click-to-acknowledgment and click-to-first-progress p50/p95/p99 separately from total elapsed deletion time. Progress must never claim that a file is removed until the operation completes.
2. **Recycle Bin first:** native `IFileOperation` with appropriate recycle flags behind a dedicated STA worker, never from GTK's hot path. Resolve shell identity and restore semantics before marking this as supported. Permanent delete must be an *explicitly separate*, confirmed choice.
3. **Fast permanent delete (opt-in only):** benchmark Win32 handle-based disposition (`SetFileInformationByHandle` / `FileDispositionInfoEx`) against `std::fs::remove_file` and native shell operations on supported local NTFS/ReFS volumes; not a global default. Windows POSIX-delete flags, read-only/ACL behavior, open handles, network shares and reparse points differ and require dedicated tests. Always reject following a junction outside the selected root.
4. **Traversal:** consider `FindFirstFileExW` with `FindExInfoBasic` / `FIND_FIRST_EX_LARGE_FETCH`, or the already efficient `std::fs::read_dir` (which reuses directory entry attributes). Test on small directories *and* 10k/100k-file trees; extra worker threads can cause lock contention and saturate HDDs or SMB. Use fixed per-volume queues, bounded-memory streaming and child-before-parent deletion. Stop for cancellations and report partial failure truthfully.
5. **Copy/move:** benchmark `CopyFile2` / `CopyFileExW` against the current 128-KiB buffered copy **inside a private staging file**, maintaining atomic no-clobber publication, error propagation and undo/rollback guarantees. Same-volume rename may use kernel handle-based rename with no-replace flags; cross-volume move requires an explicit verified copy-plus-delete contract.
6. **Navigation/search/startup:** benchmark `FindFirstFileExW` before replacing the current first-32-then-256 batched `read_dir` worker; investigate background `ReadDirectoryChangesW`/IOCP notifications and an optional USN-journal/Everything-like index for search, not for synchronous first-paint. No admin privilege or third-party service may be required for standard browsing.
7. **Long-lived process debt resolved here:** the Unix reaper once created one thread per Windows Terminal/Explorer launch. Windows now drops its process handle and does not create an unnecessary wait thread. Native GUI terminal streams stay interactive.

### Benchmark protocol and acceptance gate

Run cold and warm measurements (distinct samples) on Windows 11, NTFS SSD, HDD, ReFS if available, and remote/offline SMB. Use 0/1/1k/10k/100k files, nested directories, Unicode/extended paths, symlinks/junctions, protected/read-only files and cancellation races. Record file count, data loss (must be zero beyond the selected explicit deletion), completion/error count, memory peak, open handles, worker count, CPU, UI main-loop stalls and p50/p95/p99 end-to-end latency. Never interpret reduced *total deletion duration* as lower *first-response latency* without separate first-progress measurement. Keep benchmarks opt-in, write fixtures only to freshly created owned temporary directories, and compare with the same data and warmed cache order. CI correctness tests come before performance adoption.

**Priority:** finish Windows deletion/recycle/undo correctness first, then optimize the separately supported permanent-delete mode with measured wins; do not silently turn the safe default into irreversible deletion.
