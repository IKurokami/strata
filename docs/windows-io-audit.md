# Native I/O correctness and performance audit

Scope: `feat/windows-native-io-phase3`, before attempting the next GTK/Win32 GUI port. This report distinguishes **correctness fixes**, **structural optimizations**, and **unverified risks**. The native I/O CI compiles/tests the standalone crate, but **does not** compile or run the complete Windows GTK application.

## Bugs fixed in this audit

| Severity | Problem | Fix |
| --- | --- | --- |
| High | Old folder/preview results could remain queued and be accidentally rendered after switching destinations. | Both `try_recv` methods now discard superseded generations; deterministic regression tests inject stale events before a new request. Callers should still compare generation IDs when processing asynchronously. |
| Medium | Racing submitters could re-queue an old request after the newer pending task was consumed. | Check current generation while holding the mailbox mutex, and skip stale requests during dispatch. |
| Medium | A superseded preview could still open a file or perform the trailing truncation read. A stale directory request could still open a share. | Check generation immediately before opening, and before the final preview read. In-flight OS calls remain inherently non-cancellable. |
| High | On Windows, dot-prefixed files were treated as hidden while native hidden-attribute files were not reliably filtered in native listing and file-monitor updates. | Use cached `FILE_ATTRIBUTE_HIDDEN` metadata on Windows; an async GIO info query filters monitored upserts/moves, removing entries when they become hidden. Preserve Unix dotfile semantics. |
| Medium | A queued native file operation could accept a device namespace or a working-directory-relative path. | Reject non-absolute, non-ordinary Windows paths before I/O in the operation worker, with Windows unit tests. No UAC, Shell hooks or device probes. |
| Medium | Reserved DOS device name checks missed some special aliases and Unicode superscript COM/LPT suffixes. | Expanded lexical checks and tests, including long spaces before extensions. |

## Performance improvements

- Listing output queue: **64 → 8** batches of at most 256 names, limiting unread names to **2,048** instead of **16,384**. Old generations are drained by the consumer. Worker count remains at two for responsiveness when one operation blocks.
- Raw preview output queue: **8 → 2** results, reducing worst-case buffered raw preview data from 8 MiB to 2 MiB (plus in-flight worker data). Per-preview raw read stays capped at 1 MiB.
- Windows filename validation: remove two heap-allocated `Vec<u16>` buffers per validation and replace them with a single UTF-16 scan and a seven-code-unit stack buffer. Reserved aliases are recognized even with many trailing spaces.
- Windows directory listing: reuse Windows `DirEntry` attributes for `is_hidden` and `IconDetailsFingerprint` on cached folder revisits instead of a separate `fs::metadata(path)` for each cached entry. Linux behavior is unchanged.
- All local file operations and preview reads continue in bounded workers; no UI-thread elevation or ACL preflight has been added.

These are **structural** improvements. There are not yet credible measured p50/p95/p99 improvements for the full GUI.

## Risks remaining: do not claim fixed

1. **GUI build gate absent.** Formatting a `src/**` file is not proof the Windows GTK binary compiles or functions. Set up a real Win32 GTK toolchain and link test before port completion.
2. **Non-cancellable OS I/O.** Two workers can both get stuck on UNC paths or special driver calls. Fast cancellation rejects stale output but cannot abort a kernel call already in progress. Consider separate pools for local disks and UNC shares and a timeout watchdog with bounded worker replacement; benchmark first.
3. **File-copy failure cleanup race.** The current best-effort `remove_file(destination)` after a failed `create_new` copy is path-based. If another process replaces that path at exactly the wrong time, cleanup may remove the replacement. Do not ship this as a concurrency-safe transactional copy until cleanup is bound to the original handle or a safely committed temporary file. Existing destinations are protected at **create** time by `create_new`.
4. **Source file race.** A symlink check before `File::open` is not atomic with opening the handle. Move validation onto an opened handle or use appropriate Windows handle flags before promising no-follow semantics.
5. **No sandboxed renderer.** The raw preview worker does no untrusted format parsing. The Windows parser sandbox and UI preview integration remain pending.
6. **Linux E2E fork image.** Full application build / E2E may still depend on publishing the fork's own GHCR base image. A passing native crate matrix is not a substitute for full regression testing.
7. **End-to-end profiling missing.** Record startup-to-first-frame, first folder batch, complete folder render, selection-to-preview, memory and background I/O across local NTFS, directories with 10k–100k items, and offline/slow UNC shares.

## Verification

The standalone native crate's CI runs Windows Server 2025 and Ubuntu 24.04 matrices: Cargo metadata, Rustfmt (including application source), Clippy with warnings denied, unit tests, and release build. Full application Linux CI / Windows GUI CI have independent outstanding blockers.

No remaining port features should be merged on the claim that the native I/O crate CI alone demonstrates a working `strata.exe`.
