# Native Windows port — staged engineering plan

Status: **foundation only**, not a bootable Windows edition of Strata. Keep the Linux application, installation path, and release pipeline unchanged while platform code is extracted and integrated.

## Repository audit

- `.github/workflows/ci.yml` uses Linux/Docker build and E2E jobs; it does not validate a Windows binary.
- `.github/workflows/release.yml` produces Linux GNU binaries (x86_64 and aarch64); it does not package or sign a Windows installer.
- `.github/workflows/packaging.yml` checks Arch/AUR packaging. Do not amend the Linux release matrix until Windows builds are functional.
- `.github/workflows/pr-policy.yml` enforces conventional PR titles and examines commit metadata. Review its policy before proposing an upstream merge.
- `Cargo.toml` currently unconditionally depends on Unix/Linux-specific crates including `gio-unix`, `gdk4-wayland`, `gdk4-x11`, `landlock`, and `seccompiler`.
- `src/main.rs` and `src/model/mod.rs` import `std::os::unix`; `src/adapters/volume/mounts.rs` reads `/proc/self/mountinfo`; `src/sandbox.rs` launches Bubblewrap. This prevents a simple `--target x86_64-pc-windows-msvc` rebuild.

## Performance contract

1. **Never enumerate folders or parse previews on GTK's UI thread.** The first native-I/O module in `crates/strata-native-io` exposes small, cancellable, latest-request-wins background services. Integration with GTK remains a follow-up.
2. **Avoid synchronous security/policy probes on navigation.** Opening ordinary folders must not wait for administrator consent, UAC, an ACL walk, Shell extension execution, Defender configuration, index initialization or remote volume discovery. On inaccessible files, return a precise error asynchronously; never silently escalate privilege.
3. **Fast path like ripgrep:** avoid collecting metadata and file content during a filename-only directory listing. Show directory names in batches; fetch type, thumbnail and details only when needed. Parallelize expensive recursive work within a bounded budget.
4. **Learn from Everything without requiring its permissions:** use on-demand scans first. An optional persistent NTFS index or Windows Search integration can follow; USN journal/MFT access has volume and permission constraints and must not be a startup prerequisite.
5. **Preview only the current selection.** Coalesce superseded requests, bound raw prefetch and queued results, and isolate native content decoders in a dedicated hardened process. **Do not disable the existing Linux Bubblewrap security boundary.**
6. **Measure instead of assuming performance:** add trace spans for process-start-to-first-frame, first-visible-folder-batch, folder-settled, selection-to-first-preview, cancel-to-next-result, CPU time and peak RSS. Compare warm/cold local NTFS, network/UNC, long paths, large directories, HDD and SSD across p50/p95/p99 runs. Do not hardcode latency assertions into shared hosted-runner CI.

## Integration milestones

- [x] Create isolated Windows-port branch.
- [x] Add an independent native Windows + Linux CI gate for portable background I/O.
- [x] Implement bounded, latest-request-wins filename enumeration and raw preview prefetch services, both off the UI thread. On Windows, reuse directory entry type hints instead of per-file stat calls.
- [x] Introduce Windows-only zero-I/O path classification and safe ordinary filename validation in `strata-native-io::windows_paths`; wire native child name validation into `src/model/mod.rs` and isolate Unix-specific GVfs trash filename parsing.
- [ ] Complete OS-neutral location/URI handling throughout `src/model`, including drive/UNC, extended-length paths and remote URI edge cases; preserve non-UTF-8 Unix paths.
- [x] Add `strata-native-io` as a path dependency in root `Cargo.toml` and lockfile; validate root metadata in Linux/Windows CI.
- [x] Move Linux-specific `gdk4-wayland`, `gdk4-x11`, `gio-unix`, `landlock`, `seccompiler` and `rustix` Cargo dependencies into the Linux target section.
- [ ] Guard/adapt all remaining Linux-specific source uses with per-target implementations and prove the full GTK app compiles on Windows (not yet achieved).
- [ ] Use native Windows drive/volume enumeration, Recycle Bin, change notification and Shell integration; keep calls asynchronous.
- [ ] Integrate the native listing and preview services with existing GTK list models and verify actual first-frame behavior on Windows.
- [ ] Replace Linux-only preview isolation with a Windows restricted-token/AppContainer strategy and validate attack surface.
- [ ] Ship Windows GUI smoke tests, performance traces, and eventually a correctly bundled installer; only then add a Windows binary to release jobs.

## Native I/O service limitations

The foundation library uses only the Rust standard library and can be built and unit-tested natively on Windows. It intentionally **does not** claim to be a complete Windows filesystem backend. It lists names only (metadata is lazy), reads bounded *raw bytes* without parsing untrusted formats, does no permission escalation, and uses a bounded channel. Network I/O can still block a worker inside an OS call; two listing workers and two preview workers provide limited isolation, not a guarantee. Windows-native I/O and GUI integration need further implementation and profiling.

Run locally on Windows:

```powershell
cargo test --manifest-path crates/strata-native-io/Cargo.toml --locked
cargo clippy --manifest-path crates/strata-native-io/Cargo.toml --locked --all-targets -- -D warnings
```

To profile local directory listing on a real Windows filesystem (no privilege elevation required):

```powershell
cargo run --release --manifest-path crates/strata-native-io/Cargo.toml --example measure -- "C:\\Windows\\System32"
```

The example reports initial-pass and warm p50/p95 times for the first batch and full enumeration. This is **not** an application-startup measurement or an Everything-like indexed search. Measure on the target workstation and profile GTK integration separately.

## Validation status and remaining gaps

- Windows native I/O module unit tests, Clippy and release compilation pass on Windows Server 2025 and Ubuntu 24.04. CI includes Cargo root lockfile metadata validation, **not** full application compilation on Windows.
- In the application's `src/model/mod.rs`, Unix-only GVfs trash filename decoding now uses conditional compilation, and native Windows child names are checked against path traversal, device names and NTFS alternate data streams without I/O.
- No native Windows GUI build, running `strata.exe`, full GTK smoke test, Windows sandbox or measurable first-frame latency yet. None of these should be labeled complete.
- At this stage, the Windows classifier does not auto-resolve drive-relative (`C:foo`), root-relative (`\\foo`) or special device namespace paths; a future user-facing path handler must resolve/reject them intentionally.

## Phase 6: real Windows GTK compile diagnostics (experimental)

- `.github/workflows/windows-gui-build.yml` sets up a native UCRT64 Windows GNU Rust toolchain and installs GTK4, GtkSourceView5, GStreamer, Poppler GLib and Fontconfig from MSYS2. It invokes `cargo check --locked --no-default-features --bin strata` for the **whole application**, not just `strata-native-io`.
- The compiler diagnostic is intentionally non-blocking while the GTK build still fails; **a green workflow does not mean `strata.exe` exists**. Examine the `Compile entire GTK application for native Windows` step and its log artifact. Once the app compiles, convert this into a mandatory, fail-closed compile gate and add a native startup smoke test.
- Windows launch dispatch skips Linux-only FileChooser portal installation, GVfs probe, X11 program-class setup, and udiskie command hooks. No hidden UAC/shell elevation or Linux subprocess should be called for Windows cold startup.
- The native I/O benchmark now prints worker-construction overhead and first visible batch size plus 100 warm samples (p50/p95/p99). Neither benchmark measures GTK first paint or preview decoding; these need explicit UI instrumentation after GUI integration.
- Linux E2E had multiple sandbox/preview failures on PR #5 despite passing native I/O and Linux build/tests; rerun's attempt-scoped artifacts were unavailable. Do not label full Linux E2E as green, disable sandboxing, or skip these cases to satisfy the Windows port.

## Phase 7: Unix-only ashpd dependency isolation

- The first complete native Windows GTK compilation (phase 6) stopped at `ashpd = 0.13.13`, which imports `std::os::unix` and file-descriptor types that are not available for Win32. This was a **real compiler error**; the experimental workflow's overall green status must not be treated as a successful `cargo check`.
- `ashpd` is now a Linux-only Cargo dependency. The XDG portal server, D-Bus portal installer and Unix-specific GTK file chooser modules are compiled only on Linux. Settings already hide desktop integration controls on Windows.
- This does **not** remove the normal Windows file manager views, and does not implement native Windows file-open/save chooser integration yet. A Windows-specific chooser can be added later without XDG/Unix APIs. Linux retains the existing backend.
- Re-run the native GUI compile diagnostic and inspect its *compile step*, then address the next blocking dependency or platform-specific module. Avoid enabling unsandboxed document/media preview as a shortcut.

### Phase 7 compiler follow-up and native regression coverage

- The first Windows compile with `ashpd` excluded advanced to the application source and enumerated hundreds of remaining Unix-only call sites (notably `rustix`, `landlock`, `seccompiler`, `std::os::unix`, and GTK device handling). The observed error count on that compiler run was **556**; it is not an estimate of distinct root causes.
- Native I/O now exercises `volume_query_may_block` in the actual Windows test matrix: explicit UNC **and every absolute drive-letter path** must use the asynchronous volume-identity path, because a letter can be an offline mapped SMB drive. UI listing itself remains on its established local/UNC workers; this change governs volume identity lookups.
- GTK sidebar mount event wiring now retains portable `gio::VolumeMonitor` events on Windows without registering `gio_unix::MountMonitor`; Linux keeps both subscriptions.
- The Freedesktop XDG thumbnail cache has a deliberately **disabled Windows stub**. Windows must not use the Unix 0700/O_NOFOLLOW cache path or parse disk thumbnail content in-process. This does not provide Windows persistent thumbnail caching; implement one behind a secure native cache and sandbox boundary before claiming preview speed or feature parity.

## Phase 8: Windows UI metadata and drive boundaries

- Thumbnail metadata now uses the platform's file modification time in the existing background reader, rather than importing Unix `MetadataExt` on Win32. The persistent shared thumbnail cache remains disabled on Windows until there is a trusted native cache and isolated renderer.
- Audio cover-art versioning uses Windows creation/write timestamps and file attributes so same-size cover replacements invalidate the in-memory cache without additional probes.
- Linux device mount parsing and global-search mount discovery are compiled only for Linux. Windows global-search roots currently contain just the home folder; a future asynchronous Win32 volume enumerator can publish more roots without freezing GTK on mapped drives.
- Windows removable destinations and formatting stay unavailable until native removable-volume verification is implemented; `mkfs`, `lsblk`, UDisks and `statvfs` are not invoked on Win32. No permanent deletion is substituted for Recycle Bin.
- These changes remove several Windows compiler blockers and synchronous startup risks, but **do not** make the full Windows GTK app compile or provide Windows preview sandbox parity.

## Phase 9: Windows settings and self-update safety

- Settings change detection uses native Windows creation/write FILETIME values and file attributes (no Unix inode access), still from the existing one metadata read.
- Unix restart helpers (`sh`, `sleep`, `mv`, `/proc/PID/stat`) compile only on Linux. Windows **must not** spawn them or incorrectly restart an updated executable while the old process still holds the binary.
- The Linux inode-based rollback cleanup is disabled on Windows instead of deleting an uncertain file by pathname; Windows self-update/restart must wait for a native, verified helper protocol. This temporarily means no automated rollback cleanup/restart on Windows.
- Neither settings load nor the normal startup path should request UAC elevation, invoke a shell or perform a Windows volume permission preflight.

## Phase 10: zero-allocation action extensions and font asset paths

- File-action extension matching no longer imports Unix `OsStrExt`. It uses the portable borrowed `OsStr::as_encoded_bytes()` (no lossy conversion, heap allocation or filesystem probe in the menu hot path).
- Bundled Fontconfig font registration retains Linux's byte-safe paths and uses an explicit validated UTF-8 path on Windows. Invalid paths fail with an error rather than substituting a different font path.
- These changes remove more Win32 compiler blockers. The full GTK app remains blocked by Unix-specific sandbox, file operations and job runners; no unsafe preview fallback or unwanted elevation is enabled.

## Phase 11: native Windows atomic state persistence

- The app no longer attempts to compile `src/storage.rs` (Unix ownership, `rustix::process::geteuid`, `PermissionsExt` and `OpenOptionsExt`) on Windows. A dedicated `src/storage_windows.rs` adapter forwards to the Windows-native, independently tested `strata-native-io::windows_storage` backend.
- Writes stage with `OpenOptions::create_new` in the destination directory, explicitly flush and sync the file, release the handle, then validate and rename. Failure paths remove the staging file; no shell/UAC/Unix permission calls or TOCTOU-prone open-follow of a final symlink are used. Windows file replacement can fail if another program holds an incompatible open handle; do **not** bypass this with a non-atomic truncate or copy.
- Unlike Linux's user-owned dotfile symlinks, Windows **rejects final-component symlinks and reparse-point destinations** for both state and configuration until a verified native owner/reparse policy is implemented. Windows file ACLs come from the parent directory; the backend does not claim to have applied a private ACL. Parent junctions and ACL auditing need a separate security review before promising hardened arbitrary-path writing.
- Native Windows CI covers create/replace, refusal to replace a directory, missing parent behavior, staging cleanup and (where the runner permits creating them) symlink rejection. The full GTK Windows build and sandbox remain unresolved; the native I/O backend success is not a GUI success.
- Durable writes still involve disk I/O; use a background worker for large data or potentially remote configuration paths rather than blocking GTK. No permission preflights or Windows elevation are introduced.

## Phase 12: correct and low-latency Windows directory search

- The shallow search index uses **Windows FILE_ATTRIBUTE_HIDDEN** from `DirEntry::metadata()` (already cached by Windows FindFirstFile/FindNextFile) rather than assuming Unix dotfile naming or reading a Linux `.hidden` manifest. Dot-prefixed Windows filenames therefore remain visible unless marked hidden, matching Windows semantics.
- Windows shallow indexing no longer fetches POSIX executable modes (which have no Windows equivalent). This matches the existing deep-index worker and avoids per-file metadata probes on the path to first visible search results.
- `strata-native-io::windows_paths::is_hidden_attributes` is pure and covered by native Windows tests. The GTK application must still pass its complete Windows build and integration tests before this can be considered release parity.
- Search remains on existing background workers with cancellation and entry/time budgets; this change adds no permission prompts, shell extension calls, or foreground drive scanning.

## Phase 13: Windows custom-action runner boundary

- The Unix `src/adapters/local_jobs.rs` implementation depends on process groups, `kill_process_group`, nonblocking Unix pipes and owner-only temporary folders. On Win32 the app now compiles a dedicated `local_jobs_windows.rs` boundary instead of attempting to reuse it.
- Windows custom-action definitions remain editable/importable/exportable, but the action catalog explicitly reports them **unavailable** for execution until there is a Windows Job Object-based process-tree cancellation and containment implementation. The Windows runner reports an immediate failed event instead of silently succeeding, spawning children, or leaving an unbounded job running.
- Windows action metadata no longer calls Unix `chmod(0700)`, Unix executable-bit checks or PATH scanning on startup. Action root symlinks/junctions are rejected before directory enumeration; directory creation inherits the parent Windows ACL and **does not claim owner-only ACL enforcement** yet.
- A Windows-only GUI test exercises saving/loading a script, reporting it as unavailable, refusing to execute the script, and rejecting symlinked roots. It cannot run until the remaining unrelated GUI compiler errors are resolved; the independent native I/O CI does not test this adapter. Linux retains full custom-action execution and process-group cancellation.
- Do not turn Windows scripts on by simply replacing POSIX process signals with `Child::kill()`: child/grandchild processes can remain running, violating cancellation and potentially leaking processes or file handles.

## Phase 14: per-volume Windows mutation workers

- An absolute Windows path like `Z:\\folder` can be a disconnected mapped SMB drive even when it does not look like UNC. The native file-operation service now sends **non-system drive letters to separate, lazily created, bounded worker queues**, while keeping the OS system-drive queue and the existing UNC queue separate.
- Worker and thread creation for non-system letters happens in one **background dispatcher**, not in `submit()` on GTK's UI thread. Drive letter identification uses the path prefix and cached `SystemDrive` environment value only; **no GetDriveType, volume enumeration, ACL checks, shell handlers or UAC** are performed on the submission path.
- Each per-letter worker preserves ordering for its own volume, keeps at most eight waiting mutations and reports queue exhaustion as an error completion. A failed worker spawn also completes the accepted request with an error. An offline mapped drive can still stall its own worker, but not unrelated drive queues. Up to 25 non-system lanes can be created lazily when actually used.
- A Windows-native behavior test simulates a blocked mapped-letter worker **before any filesystem call**, verifies a real create-directory operation on another local drive finishes independently, then cancels the fake mapped request before releasing it. No I/O is attempted on the fake drive.
- This improves the existing native operation backend only. **Integration of its completion queue into the full Windows GTK operation provider is still pending**, as are Recycle Bin, no-clobber move/rename and a safe archive backend. Do not interpret this backend test as the full file-manager GUI running.

## Phase 15: GTK operation provider → native Windows workers

- `src/adapters/local_operations_windows.rs` replaces the Unix/POSIX `local_operations.rs` module on Windows, removing its Unix file descriptors, `rustix` ownership/mount flush assumptions, Linux archive mutation and direct delete/Trash helpers from the Windows compiler path.
- Supported in this staged provider: native worker **create folder** (explicit name) and **copy one regular file** with `FailIfExists`, plus cancellable GIO no-overwrite **new file** and collision-retrying **unique new folder**. These are operations whose destination identity or atomic no-clobber rules can be tracked without a Windows Recycle Bin.
- A single lazily initialized `LocalOperationService` per provider is shared by requests, preserving the bounded local/UNC/mapped-drive queues from previous phases. GTK polls completed operations at most once per 16 ms and invokes all UI callbacks on the GTK thread. `LoadHandle` cancellation removes the pending listener and asks the native service to stop before publication; stale completions cannot update a closed view.
- Unsupported actions (move/rename, overwrite, `KeepBoth`, batch paste, merge, permanent deletion/Recycle Bin, undo, archive compress/extract) explicitly report errors. **No irreversible operation is silently substituted**; the current partial GUI port does not claim feature parity.
- Archive metadata wire decoding is retained but **direct archive reads, decoders and extraction are disabled** on Windows until the preview sandbox/worker boundary is implemented; unsafe in-process decoding is not a substitute.
- Linux continues to build the original `local_operations.rs` code with its existing archive fixtures and mount behavior. Existing Windows native operation tests still run independently; GTK-specific operation bridge tests will require a successfully compiled Windows GUI before they can run. Benchmark figures from the backend do not measure first GTK paint.
- Lazy service initialization happens on the first supported user operation (never at app launch); thread startup is a bounded in-memory operation, while all actual disk and UNC mutation work remains on background workers. An end-to-end Windows performance run is required before claiming UI latency budgets.

## Phase 16: Windows updater and installation-source isolation

- Native Win32 builds no longer compile the Linux self-update implementation: it probes `/proc`, retires instances via PID FDs/signals, reads Unix executable ownership, runs pacman/AUR helpers and uses Linux rename/rollback semantics. These are not valid substitutes for a verified Windows installer.
- Windows `install_update` **always rejects** self-replacement with a visible `Failed` result and does not download, write, launch a shell, request UAC, terminate processes or create rollback backups. A pre-cancelled request reports `Cancelled`. It keeps the shared event/request API, and update checks may still find newer releases; the UI's InPlace choice does not indicate installation is supported on Windows.
- Linux-only install-source discovery and `PATH` executable permission probes are skipped on Windows first-window startup. `current_exe` stays available without guessing a Linux ` (deleted)` suffix.
- `TrashedOriginal::at_path` returns `None` on Windows rather than manufacturing POSIX device/inode identities for Recycle Bin operations. Windows delete/restore remain fail-closed in the operation provider.
- Windows-only GUI unit tests cover updater refusal and cancellation but cannot run until the remaining sandbox/media compiler blockers are fixed. The independent native I/O CI cannot validate this GUI-level code. A future update helper needs publisher signature verification, safe close-and-restart, rollback and no elevation during ordinary file-manager use.

## Phase 17: native Windows external-provider boundary and startup performance

- On Windows, opt-in external file providers are disabled until a trusted Win32 named-pipe transport, verified ownership/ACLs and Job Object containment exist. No Unix sockets, `CommandExt::pre_exec`, `rustix` FD probes or uncontained provider helpers may run.
- The existing metadata protocol and GUI client interfaces are retained so UI code compiles unchanged. Provider discovery returns no registrations, and a synthetic client always refuses requests with a disconnected result; it never reports a fake successful activation. Unit tests cover empty registration and explicit refusal (GUI tests cannot run until independent Win32 compiler blockers are resolved).
- Windows first-window construction no longer starts a provider discovery thread **or the old 100 ms GTK polling timer** when no provider can possibly exist. Linux continues the original opt-in scanning and transport with its existing security checks and tests.
- A future Windows provider implementation needs to validate the publisher and binary identity without resolving arbitrary user-supplied executables from PATH; operations and cancellation must remain off GTK's main thread and must be able to stop all descendants.

## Phase 18: native Win32 folder selection and Recycle Bin restore isolation

- Windows destination selection uses asynchronous GTK4 `FileDialog::select_folder_future`, which delegates to the platform chooser only **on explicit user request** (not at startup). It supports transfers' existing destination callbacks without xdg-desktop-portal, Unix FDs, extra chooser process or elevation.
- For constrained removable roots, it resolves the root and the selected folder on a background thread before invoking the destination validator. Windows junctions and directory links cannot escape confinement by simply passing a lexical-prefix check; the transfer action still revalidates its destination just before dispatch. Normal selections retain their original validation behavior.
- Linux freedesktop.org `.trashinfo` restore path guessing is **not** treated as a Windows Recycle Bin implementation. All Windows restore planners return an explicit unsupported error and never mutate the disk or manufacture inode/UID identities. A future Windows implementation needs trustworthy shell-item identifiers, safe original-path validation and undo guarantees.
- Embedded document-image rendering is disabled on Windows until a secure native open-beneath/junction-aware resolver and isolated parser exist, preventing a malicious Markdown image path from escaping the document folder. Linux still uses `openat2(RESOLVE_BENEATH|NO_MAGICLINKS)`.
- The Windows GUI remains incomplete and this change does not bypass preview sandboxing. Any GUI tests in these new modules cannot run until the full application compiles; Linux and native I/O CI check independent behavior.

## Phase 19: explicit native Windows sandbox boundary

- On Windows, **none** of the Linux bubblewrap, Landlock, seccomp, PidFD or Unix socket helper code is compiled or invoked. `sandbox_windows.rs` keeps the app-facing cancellation, preview and media model types, but **fails closed** for untrusted image/PDF/media/model/archive decoding until a real Win32 sandbox is available.
- A browser thumbnail request returns a concrete unavailable result rather than invoking a fallback in-process decoder or an uncontained subprocess. Media sessions likewise refuse startup; metadata wire parsers remain available for future trusted helper results.
- `--preview-helper` and `--browser-worker` cannot be used on Windows to bypass isolation. A future implementation requires a Job Object (kill-on-close), bounded I/O and memory, restricted launch token/AppContainer when appropriate, pipe identity verification, and cancellation of all descendants.
- The shared decoded-media wire structures remain compiled on Windows; only the Linux FD/poll-based timed reader is excluded.
- **Functionality trade-off:** advanced thumbnail/preview decoders are temporarily unavailable on Windows. Core file navigation, listing and safe file actions remain separate and should continue to work as they are integrated. This is not full feature parity, and no untrusted parser should be enabled without equivalent containment.

## Phase 21: reproducible Windows developer distribution

- `scripts/windows-bundle-ucrt64.sh` recursively inspects PE imports with `objdump`, resolves only trusted build-time UCRT64 runtime DLLs from `/ucrt64/bin`, and stages GTK/GLib icon, schema, MIME and optional native plugin resources. It deliberately never copies the MSYS2 POSIX runtime, Cargo, Rust toolchains or arbitrary PATH binaries.
- `Run-Strata.cmd` sets application-local library/schema paths without installation, registry modification, UAC, shell extension registration or any application startup drive scanning.
- A new **blocking Windows Server 2025 CI** builds the GTK executable, assembles the folder, then launches both the bundled binary and launcher in PowerShell with MSYS2 removed from PATH. It uploads an experimental developer artifact **only when the isolated runtime check passes**.
- Passing `--version` proves that the Windows loader can resolve PE runtime imports outside MSYS2's PATH; it does *not* verify first GUI paint, GStreamer plug-in coverage, Explorer integration or feature parity. The README explicitly discloses missing features and no publisher code-signing.
- This is a staging step toward a tested portable zip. After successful CI, add an interactive Windows desktop startup test, measured cold folder navigation and checks for loader/icon plug-ins before considering a public release.
