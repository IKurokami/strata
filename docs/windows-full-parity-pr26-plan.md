# PR #26 — Complete Windows parity, security, performance, and release readiness

> **Single-PR execution contract for the Windows AI coding agent.**
> This document is a roadmap, not a claim that any of the work below is implemented.

## Branch and integration policy

- Source repository: `IKurokami/strata`; upstream: `lgse/strata`.
- **One subsequent PR only:** add ALL remaining Windows feature parity, security fixes, performance engineering, and release hardening to `feat/windows-full-parity-and-performance`. Use logically scoped commits, but do not open separate feature PRs.
- This branch starts at PR #25 head `8fa439bc3419d63c5b0100cc81a3868fac4cbe12`. While PR #25 is open, **base this PR on `feat/windows-shell-integrations-20261014`** so the diff contains only new work.
- PR #25 already incorporates PR #24 via a merge commit. **After PR #25 is merged into main, retarget this PR to `main`**, update/rebase carefully if needed, without discarding fork commits. Do not independently merge duplicate #24 code.
- PR #25 head had all four CI workflows green at branch creation (Windows native I/O, Windows GTK compilation, Windows linked executable smoke, Linux CI/E2E). Preserve that baseline.
- Keep Linux behavior and its release CI intact, with appropriate `cfg(windows)` and `cfg(unix)` boundaries.
- All desktop user-facing capabilities present on Linux should have safe Windows equivalents where technically possible. Only Linux-exclusive *infrastructure* (Wayland, D-Bus, UDisks2, Bubblewrap etc.) may be left Linux-only; their product capabilities still require Windows equivalents.
- Do not ask the user to approve ordinary engineering steps. Implement, test, commit, push, review CI and continue within this same PR. Do not claim that a green `cargo check` is acceptance.

## Execution sequence inside this ONE PR

### Gate 0 — Preserve correctness, establish real Windows acceptance

- [ ] Read the entire PR #25 diff and all open/resolved comments from PR #24/#25, plus `docs/windows-parity-audit-2026-10-14.md`, `docs/windows-io-audit.md`, `docs/windows-development.md` and `docs/windows-port.md`.
- [ ] On Windows 10/11, install MSYS2 UCRT64/GTK4, run `./scripts/windows/dev.ps1 build`, `test`, `run`, `package`. Update `scripts/windows/dev.ps1 test` so it includes the new rename/move/batch-copy tests that the GitHub linked-executable workflow already includes.
- [ ] Validate real interactive first paint; keyboard, mouse, folder navigation, context menu, terminal, copy path, clipboard, tabs, settings and save/restart on Windows. Do not infer GUI readiness from `--version` smoke.
- [ ] Verify source/created location contracts and undo bookkeeping across `OperationEvent::Pasted`, `TransferProgress`, `ItemMoved`, cancellation, partial failures and background operations.
- [ ] Run cross-platform format, lint, unit tests, Linux E2E and native Windows tests; repair underlying failures, including intermittent race and accessibility failures, without weakening tests.
- [ ] Add automated Windows interactive GUI smoke/E2E and upload logs and screenshots on failure, while isolating tests from the user's real filesystem.

### Gate 1 — Finish every core file operation (P0)

Primary implementation: `src/adapters/local_operations_windows.rs`, `crates/strata-native-io/src/windows_operations.rs`, `src/services/operations.rs` and browser operation events.

- [ ] Correctness audit of PR #25's asynchronous rename/undo rename; test case-only NTFS renames, Unicode, names with trailing dots/spaces, path aliases, long paths, locked items, collision races, directory trees and cancellation after OS completion.
- [ ] Implement recursive directory copy (including empty dirs) and batch copy with accurate byte/file progress, metadata preservation as required, bounded buffers, staging, verified no-clobber publish, crash cleanup and partial-error recovery.
- [ ] Complete same-volume batch move/undo, plus **cross-volume move with safe copy/publish/verify/delete** (source must survive an uncommitted destination failure). Handle FAT32/exFAT/NTFS/ReFS/SMB capability differences.
- [ ] Implement collision policies `FailIfExists`, `ReplaceExisting`, `KeepBoth`, `Merge`, including rename/copy/move/drag/drop/clipboard paths.
- [ ] Implement reversible overwrite and directory merge, full undo/redo (copy, merge, rename, move, delete), identity-checked cleanup and correct handling of half-completed batches.
- [ ] Include concurrency/fault-injection regression tests for destination created mid-operation, source swapped, reparse-point/junction escape, disk full, access denied, locked files, malformed Unicode, offline UNC, removable disconnect and cancellation races.
- [ ] Never silently convert a copy into an overwrite, or a cross-volume move into an unverified copy followed by source deletion.

### Gate 2 — Native Windows Recycle Bin and safe deletion (P0)

Primary implementation: `src/adapters/trash_restore_windows.rs`, `src/adapters/local_operations_windows.rs`.

- [ ] Default Delete = Windows Recycle Bin on supported volumes; restore with verified original path/shell identity; explicit conflict handling; undo delete and undo copy; support batches.
- [ ] Use native Windows Shell API (evaluate `IFileOperation`) on an isolated COM STA worker, **never** on the GTK UI hot path; provide genuine progress, cancellation, partial failure and error details.
- [ ] Explicit permanent deletion must be a separately confirmed mode. No silent Recycle Bin → irreversible delete fallback.
- [ ] Evaluate `SetFileInformationByHandle`/`FileDispositionInfoEx` for opt-in fast permanent deletion, but only with accurate permissions, reparse-point policy, cancellation semantics, bounded concurrency and safe handling of open files.
- [ ] Study `suptejas/delete` and `suptejas/turbo-delete` for algorithmic ideas only. Do not import their unbounded traversal, `unwrap()`, swallowed errors or `follow_links(true)` behavior. Do not traverse outside the selected deletion root.
- [ ] Test 0/1/1k/10k/100k files, nested dirs, symlinks/junctions, read-only/locked files, Windows Recycle Bin per volume, restoration identity conflicts and cancellation.

### Gate 3 — Safe preview, thumbnails and archives (P0)

Primary implementation: `src/sandbox_windows.rs`, `src/sandbox/browser_windows.rs`, `src/sandbox/media_windows.rs`, `src/sandbox_helper_windows.rs`, `src/ui/thumbnail_cache_windows.rs`.

- [ ] Build an authenticated, bounded, cancellable out-of-process Windows decoder. Evaluate Windows Job Objects, restricted tokens/AppContainer and defensible filesystem/network containment. Never parse untrusted PDF/media/archive in the unrestricted GTK process.
- [ ] Re-enable image, RAW where Linux supports it, PDF, text/Markdown/source code, documents, spreadsheet/CSV, audio and video previews; include content sniffing by magic bytes where appropriate.
- [ ] Add efficient thumbnail generation/cache, corruption/crash containment, invalidation based on trustworthy file identity, latest-selection-wins, bounded memory/process lifetime and no decode during startup.
- [ ] Port archive browse/compress/extract (ZIP, 7z, TAR/TAR.GZ, optional RAR) with passwords, progress/cancellation, safe staging, no path traversal, no zip bombs, no untrusted symlink escape, and atomic conflict policy.
- [ ] Preserve the Linux sandbox and all Linux preview behavior.

### Gate 4 — Complete Windows integration and product parity (P1)

- [ ] `src/adapters/volume/mounts_windows.rs` and `mount_snapshot_windows.rs`: asynchronous volume enumeration, drive types, USB/removable status, network/mapped drives, unique identities, mount notifications, safe flush/eject. No synchronous remote volume probes on GTK.
- [ ] `src/services/file_providers_windows.rs` and `src/services/file_providers/transport_windows.rs`: on-demand SFTP/providers using authenticated Windows IPC (named pipe peer security/ACLs) or equivalent, with workers properly isolated and cancelled. Offline network paths must not block local navigation.
- [ ] `src/adapters/local_jobs_windows.rs`: custom actions with safe command construction, working-directory and process-tree control via Windows Job Objects; no shell interpolation or unintended privilege elevation.
- [ ] `src/services/update_install_windows.rs`: signed/verified native updater, safe exit/restart and rollback. Build a Windows installer or correctly self-contained portable release.
- [ ] Verify shell integrations (Explorer show/reveal, Terminal fallbacks, Open With, clipboard/drag-drop, Unicode/UNC). Add optional, opt-in Windows shell context registration, properties, shortcuts (.lnk), WSL interoperability, DPI/theme/taskbar integration when safe and useful.
- [ ] Review upstream changes in `lgse/strata`; selectively port missing fixes (previous audit candidates: `c0490319`, `586ca3c9`, `59c67222`, `6aa721f7`, `58981b33`, `c85a001d`) without duplicating already cherry-picked equivalent code.

### Gate 5 — Deep performance, stability and security engineering (P1/P2)

- [ ] Establish cold/warm baselines on Windows 11: process start → first interactive frame; click folder → first 32 entries → settled; selection → first preview; copy/move/delete first acknowledgment/progress and total throughput; search first result; cancellation responsiveness; p50/p95/p99, CPU, memory, handles, thread count and UI frame stalls.
- [ ] Compare `std::fs::read_dir` with `FindFirstFileExW`/`FindExInfoBasic`/`FIND_FIRST_EX_LARGE_FETCH` before rewriting enumeration. Preserve latest-request-wins, streaming 32→256 name batches, and bounded queues.
- [ ] Evaluate `ReadDirectoryChangesW`/IOCP watchers; optional USN/MFT/Everything-like indexing with permission-aware fallback. Do not require a background indexer or administrator access for normal navigation.
- [ ] Benchmark staged `CopyFile2`/`CopyFileExW` against current 128 KiB copy and Win32 disposition delete against safe alternatives; use per-volume bounded concurrency, not unbounded threads.
- [ ] Audit GTK main-loop blocking and the current 16 ms completion poll and 64-completions/3 ms budget; profile before considering event-driven wakeups. Optimize hot-path allocations, sorting, list model updates, thumbnails, stale callbacks, startup DLL loads and cache churn only when measurable.
- [ ] Fix technical debt: `unwrap`/`panic` on user paths, RefCell reentrancy, resource leaks, stale generation work, process/thread crashes, race conditions, path traversal, reparse-point TOCTOU, non-UTF8 filename handling, Windows privileges, network stalls and crashes.
- [ ] Do not sacrifice correctness, undo or security for benchmark wins. Benchmark SSD/HDD and mapped/offline SMB separately.

### Gate 6 — Final Windows release acceptance

- [ ] All user-visible Linux functionality that can be safely implemented has a working native Windows path; no remaining `unsupported` for these capabilities.
- [ ] Nonportable Linux-only infrastructure is excluded only on Windows and Linux regression tests remain green.
- [ ] Windows app passes **actual GUI interaction E2E**, not merely executable compilation/`--version`.
- [ ] All Windows native I/O, Windows GUI compile, linked-executable + filesystem regressions, Linux format/lint/unit/E2E and packaging workflows green **on the same final PR head SHA**; no skipped/flaky tests.
- [ ] Windows portable/installer works on a clean non-development Windows computer without MSYS2 installed or in PATH.
- [ ] Explicitly verify no unintended deletion/data loss, sandbox escape, known critical crash, unhandled partial failure, handle leak or unbounded background work.
- [ ] Publish baseline and before/after benchmark results, compatibility matrix, security review, remaining hardware-only limitations and final CI URLs in PR description.
- [ ] Resolve review threads and comments, recheck newly introduced issues, then mark PR ready for review/merge. Keep PR as **Draft** until the implementation is genuinely complete.

## Workflow for AI agent

1. Run real local Windows build/test/GUI acceptance.
2. Choose the next unmet task in dependency order above.
3. Implement with relevant unit/integration/E2E and failure-injection tests.
4. Run `cargo fmt --all -- --check`, Clippy, full Rust tests and Windows GUI tests.
5. Commit with a scoped message; push **only to this PR branch**.
6. Review GitHub Actions, review threads, source-level regressions and reported defects. Fix until the latest commit passes.
7. Keep an up-to-date checkbox ledger in this document with concrete code/test evidence for each completed item. Do not check a box based on an intent or a plan.
8. Do not start a new PR to split the work. Continue until all realistic, safely portable functionality, performance measurements and release gates are complete.

**Current implementation status:** PR #25 completed the foundation (rename/undo rename, same-volume move/undo, batch no-clobber file copy, Windows shell launch/clipboard, portable packaging, GTK build and CI). This branch now also has native Shell Recycle Bin restore/permanent-delete workers, recursive no-clobber directory copy, sandboxed archive/media helper execution, cached background removable-volume discovery, bounded opt-in provider discovery/Job Object transport, geometry-preserving media playback, title-bar ownership, and timeline handoff regression coverage. The branch still has explicit parity gates outstanding: collision policies, cross-volume move fault injection, full archive mutation coverage, authenticated provider ACLs, interactive Windows GUI E2E, clean-machine installer acceptance, performance/security evidence, and same-SHA CI/review approval.
