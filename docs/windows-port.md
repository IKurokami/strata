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
