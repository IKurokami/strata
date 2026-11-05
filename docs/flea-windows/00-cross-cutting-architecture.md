# Kiến trúc dùng chung cho 23 tính năng

## Giữ nguyên Strata Windows trước khi mở rộng
Strata sử dụng GTK4/GIO + Rust domain/application/service/adapters, Windows helper crates và sandboxed media workers. [Source architecture](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/architecture.md). **Không thay toolkit**, không tách riêng file manager thứ hai. Dùng shared Browser/selection/operations, chỉ bổ sung capability boundaries nơi có hai implementation thật.

### Sơ đồ đề xuất
```text
GTK4 Window
  ├─ TabSession[] — mỗi tab 1 hoặc 2 BrowserSession độc lập
  │     ├─ BrowserPresentation: Columns / Icons / List / Disk Usage
  │     ├─ SearchSession (filename | indexed content | live content)
  │     ├─ PreviewSession (inline drawer | Quick Look overlay)
  │     └─ Focus, Selection, History, PathIdentity
  ├─ Command Dispatcher + Keymap Profile (Windows | 10xer | custom)
  └─ Progress Dock and user-confirmation dialogs
            │ typed intents, generation IDs, no synchronous I/O
Application Services
  ├─ OperationQueue + ConflictPlan + OperationJournal
  ├─ PreviewRegistry + RendererHelper
  ├─ Search Services + GitStatusSnapshot
  ├─ Transforms (rename/compare/convert/compose/print)
  └─ ProviderRegistry (existing file providers + scoped extensions)
            │
Windows Adapters
  ├─ Local File I/O (NTFS/ReFS/exFAT/FAT/UNC, reparse checks)
  ├─ Job-supervised helpers (cancellation/time/memory/output limits)
  ├─ Win32 printing, Shell, cloud/remote, Windows Open With
  └─ AppData configuration / cache / optional indexes
```

## Công thức callback/state bắt buộc
Đặt RequestId và generation theo **tab + pane + document/file identity**, không dùng path string làm identity duy nhất. Khi chuyển folder/đóng pane/đóng tab/đổi preview: cancel worker, increment generation, drop result trễ. Một file operation được commit một lần qua OperationQueue; UI chỉ hiển thị progress/outcome, không viết filesystem từ GTK callback. Ưu tiên trả tên/metadata sớm, materialize detail/thumbnail/decoder theo viewport. Mọi blocking filesystem, shell, Git, printer/network call đều chạy worker.

Dữ liệu dùng native PathBuf/OsString; không ép tên file Windows sang UTF-8 trong lõi. UI phải có localized display string và fallback; protocol JSON dùng explicit conversion hoặc encoded path identity có validation. Tránh giữ lock khi gọi GTK hoặc chờ subprocess; đóng view không join thread vô hạn.

## File mutation transaction
1. Lấy immutable selection snapshot (source identity, source path, target directory, mtime/size/change token).
2. Planner validate: duplicate input/outputs, case-fold collision, reserved Windows names, conflicts, reparse points, ACL and access, per-volume rules; show preview/dry-run nếu destructive.
3. Execute ở background: bounded parallelism, fresh private staging on destination volume, no-clobber publish; never assume rename atomic on remote/cross-volume.
4. Verify result and append journal **only for confirmed steps**; rollback best-effort on error/cancel, report exact partial state. Preserve existing Windows undo/recycle semantics; never map recycle failure to permanent delete.
5. External changes invalidate plan; recheck immediately before publish and before deleting source.

## Security: boundaries, not buzzwords
- Windows Job Objects: process-tree lifetime/CPU/memory caps, kill-on-close, output/time caps. **A Job Object is not a security sandbox**. For parsers/plugin previewers that process hostile bytes, require reviewed low-privilege/AppContainer/restricted-token worker and constrained file access, or fail closed. Existing Strata helper can be reused for limits, but privilege isolation must be verified separately.
- Third-party custom actions/providers are explicitly **trusted user programs** (per existing docs), so UI must disclose file access. No arbitrary DLL plugins, command-line interpolation, untrusted dynamic code in GTK process.
- Staging directories created with restrictive ACL, unique names; path resolution must prevent symlink/junction/reparse traversal, TOCTOU, archive zip-slip and network destination escape. Never follow untrusted links merely because the UI displayed them.
- Treat remote/share/cloud as different capability levels: latency, rename semantics, file identity, ownership and atomicity can be unavailable; decline unsupported operations rather than fake success.
- No network by default for local previews, no auto-upload of documents/paths/search queries. Users explicitly opt into cloud destinations and semantic model downloads.
- Printing, export, cloud send, replacement, batch rename are explicit user actions with predictable confirmation/error paths.

## Windows adapter choices
- Native file identity via volume identity + FILE_ID_INFO when available, validated on reopen. On UNC/non-NTFS where stable identity unavailable: fall back to path + revision and mark as weaker; never silently join identities.
- Directory watcher: ReadDirectoryChangesW overlapped worker, generation IDs, overflow/error or disconnected UNC → bounded full rescan; never block GTK.
- Git status: git.exe porcelain v2 -z per repository snapshot in supervised worker; read-only status, timeout/byte cap; do not spawn per-row Git.
- Printing: native Windows PrintDlgEx/Spooler and type-specific rendering/associated app fallback; never shell-interpolate path.
- SMB: native UNC path and Win32 error handling; SFTP: on-demand Windows-compatible provider/transport with host-key validation and no credentials in UI logs.

## Shared test/evidence contract
Test each RFC at three levels: pure planning/protocol fixture; headless worker with injected timeout/cancellation/collision; Windows GUI accept on clean Windows 10/11, local NTFS and UNC/offline fixture. Important negative cases: 100k files, 2+ GB file, inaccessible paths, long Unicode/emoji/case-only names, locked file, symlink/junction/reparse, FAT32/exFAT, read-only share, race after preview, cancel during copy, process kill, archive traversal, shell/plugin crash. Never assert GUI screenshot geometry in Rust layout tests (repo AGENTS rule); use manual captures for visual acceptance.

Capture (opt-in diagnostics) generation, duration bucket, cancellation reason, bytes and counts, not paths/content/credentials. Write exact test commands and measured numbers once implementation exists; these docs are **design targets**, not successful tests.

## Contracts that MUST remain intact
[Keyboard routing](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/keyboard-navigation.md), [preview layout](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/preview-panel-layout.md), [actions](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/custom-actions.md), [Windows security](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/windows-security-review.md), [saved settings](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/preferences.md). Khi thực thi phải i18n đủ 11 locale; không đưa hardcoded hotkeys ngoài ui::keys::bindings.
