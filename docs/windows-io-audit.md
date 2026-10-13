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
| High | Failed copies could expose partial output or risk clobbering a concurrently created destination. | Stage bytes under an exclusive scratch name, publish via atomic no-overwrite NTFS hard link only after complete write; tests cover interrupted source and concurrent destination. Source is validated via the opened handle. |

## Performance improvements

- Listing output queue: **64 → 8** batches of at most 256 names, limiting unread names to **2,048** instead of **16,384**. Old generations are drained by the consumer. Worker count remains at two for responsiveness when one operation blocks.
- Raw preview output queue: **8 → 2** results, reducing worst-case buffered raw preview data from 8 MiB to 2 MiB (plus in-flight worker data). Per-preview raw read stays capped at 1 MiB.
- Windows filename validation: remove two heap-allocated `Vec<u16>` buffers per validation and replace them with a single UTF-16 scan and a seven-code-unit stack buffer. Reserved aliases are recognized even with many trailing spaces.
- Windows directory listing: reuse Windows `DirEntry` attributes for `is_hidden` and `IconDetailsFingerprint` on cached folder revisits instead of a separate `fs::metadata(path)` for each cached entry. Linux behavior is unchanged.
- On Windows, UNC listing and preview requests now use dedicated bounded worker mailboxes that share the generation counter with local workers. An unreachable UNC directory cannot consume the two local directory/preview workers. Classification is lexical, without synchronous volume probing.
- All local file operations and preview reads continue in bounded workers; no UI-thread elevation or ACL preflight has been added.

These are **structural** improvements. There are not yet credible measured p50/p95/p99 improvements for the full GUI.

## Risks remaining: do not claim fixed

1. **GUI build gate absent.** Formatting a `src/**` file is not proof the Windows GTK binary compiles or functions. Set up a real Win32 GTK toolchain and link test before port completion.
2. **Non-cancellable OS I/O remains on the network pool.** Explicit UNC paths now have one dedicated worker each for listing and preview, so a stuck UNC operation does not consume the local workers. A second UNC request can still queue behind a stuck call, and a mapped network drive looks like a local drive to lexical classification. Real cancellation and mapped-drive detection are separate Win32 tasks; do not automatically spawn unbounded replacement workers.
3. **Copy atomic publication implemented, but platform limits remain.** Copy creates an exclusive scratch file in the destination directory and publishes through `fs::hard_link` only after all source bytes are read and writer is flushed. Neither interrupted reads nor concurrent creation expose a partial final destination, and final names are never overwritten. NTFS supports this primitive; other volumes (including some network shares) can reject hard links, and the caller must report the failure instead of falling back to unsafe rename/copy. Scratch cleanup is currently best-effort by a high-entropy pathname; handle-bound deletion under adversarial directory modification is a remaining hardening item.
4. **Source final-component race reduced.** Windows raw-preview and copy open the actual source handle with `FILE_FLAG_OPEN_REPARSE_POINT` and validate attributes on that opened handle. A final-component symlink or reparse point is rejected, even if swapped before opening. Intermediate parent junctions can still be followed and require separate directory-handle traversal if the threat model warrants it.
5. **No sandboxed renderer.** The raw preview worker does no untrusted format parsing. The Windows parser sandbox and UI preview integration remain pending.
6. **Linux full CI/E2E remains a distinct gate.** Repaired earlier errors from scoped Linux settings controls, changed monitor test signatures, and the missing in-repository crate in the E2E Dockerfile and `.dockerignore`. These changes require their own clean, completed CI run; a passing native crate matrix is not a substitute for full application regression testing.
7. **End-to-end profiling missing.** Record startup-to-first-frame, first folder batch, complete folder render, selection-to-preview, memory and background I/O across local NTFS, directories with 10k–100k items, and offline/slow UNC shares.

## Critical mitigation tests added

The Windows native I/O crate has executable regression cases for:

- A mocked source read that fails after the first byte: the final filename never appears and scratch cleanup is attempted.
- Another process creating the final name during copying: the new file is preserved and publication fails with `AlreadyExists`.
- Rejection of directory/device/relative paths and reserved Win32 child names without writing to them.
- Shared generation across isolated local/UNC mailboxes and cancellation of stale preview/listing events.

These tests validate the worker and filesystem publication guarantees, not a sandboxed GTK renderer. The Windows preview worker remains a **raw-byte reader only**; untrusted document/media parsing must remain disabled until out-of-process isolation is available.

## Phase 4: independent network mutation queue and explicit cancellation

- `LocalOperationService` now has distinct bounded local and UNC mutation queues. Direct UNC sources **or** destinations route to the remote worker, so a stuck copy on an offline share does not delay local create-directory/copy requests. Accepted tasks remain FIFO **within each queue**; cross-queue completion order is not guaranteed.
- The remote mutation queue holds at most 8 pending requests. No retry thread is spawned when a UNC kernel call hangs; existing operations report `Busy` if their queue is saturated. Mapped network drive letters still cannot be classified from path syntax alone.
- `cancel(id)` cooperatively interrupts a queued or active file operation. Active copy checks the atomic cancellation flag before each 128-KiB read and immediately before atomic no-overwrite publication. A kernel read or create-directory syscall already in progress **cannot** be forcibly cancelled. There is no guarantee an operation can be rolled back after final publication.
- The bounded outstanding-operation map contains only accepted requests and is cleared on worker completion or queue rejection. This adds an allocation and a short mutex lock per file mutation, not to the hot filename-enumeration path.
- Cancellation unit tests use a deterministic reader to set the flag mid-stream and assert that no final destination name is ever published.

**Security boundary:** cancellation and raw-byte prefetch are not a decoder sandbox. No PDF, media, image, or executable parser may be enabled in-process on Windows merely because these file-operation safeguards are present.

## Verification

The standalone native crate's CI runs Windows Server 2025 and Ubuntu 24.04 matrices: Cargo metadata, Rustfmt (including application source), Clippy with warnings denied, unit tests, and release build. Full application Linux CI / Windows GUI CI have independent outstanding blockers.

No remaining port features should be merged on the claim that the native I/O crate CI alone demonstrates a working `strata.exe`.
