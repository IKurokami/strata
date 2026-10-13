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
- [x] Implement a bounded, latest-request-wins filename enumeration service and raw preview prefetch primitive, both off the UI thread.
- [ ] Extract OS-neutral location/path semantics from `src/model` (Windows UTF-16 and drive/UNC handling; preserve non-UTF-8 Unix paths).
- [ ] Split Cargo dependencies and implementation modules using per-target `cfg`, while preserving the supported Linux build.
- [ ] Use native Windows drive/volume enumeration, Recycle Bin, change notification and Shell integration; keep calls asynchronous.
- [ ] Integrate the native listing and preview services with existing GTK list models and verify actual first-frame behavior on Windows.
- [ ] Replace Linux-only preview isolation with a Windows restricted-token/AppContainer strategy and validate attack surface.
- [ ] Ship Windows GUI smoke tests, performance traces, and eventually a correctly bundled installer; only then add a Windows binary to release jobs.

## Native I/O service limitations

The foundation library uses only the Rust standard library and can be built and unit-tested natively on Windows. It intentionally **does not** claim to be a complete Windows filesystem backend. It lists names only (metadata is lazy), reads bounded *raw bytes* without parsing untrusted formats, does no permission escalation, and uses a bounded channel. Network I/O can still block a worker inside an OS call; two listing workers provide limited isolation, not a guarantee. Windows-native I/O and GUI integration need further implementation and profiling.

Run locally on Windows:

```powershell
cargo test --manifest-path crates/strata-native-io/Cargo.toml --locked
cargo clippy --manifest-path crates/strata-native-io/Cargo.toml --locked --all-targets -- -D warnings
```
