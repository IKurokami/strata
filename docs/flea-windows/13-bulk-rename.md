# 13 — Bulk rename / đổi tên hàng loạt

**Trạng thái Strata (source review): THIẾU NATIVE UI** · **Ưu tiên: P0** · **Design-only — chưa có implementation/Windows GUI acceptance mới**.

## Flea thực sự triển khai thế nào?
Flea nhận mapping tên cũ→mới, validate whole-batch, dùng tên tạm '.flea-bulk-*' giải vòng hoán đổi và ghi **một entry undo**. GPUI dialog preview trước, backend tránh overwrite. Một vài ví dụ rename của Strata hiện tại chỉ là Python script Linux renameat2, không phải Windows-native batch.

### Bằng chứng Flea ghim commit 73f3c239
- [bulkrename.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/bulkrename.rs)
- [renamecompat.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/src/backend/renamecompat.rs)
- [bulkrename.rs](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/crates/flea-gui/src/g2/op3/bulkrename.rs)
- [write-operations-and-the-undo-journal.md](https://github.com/thisisgm/flea/blob/73f3c23971106f6d47f78bd758f783af3e5b5c78/docs/architecture/write-operations-and-the-undo-journal.md)

## Hiện trạng và khoảng trống của Strata Windows
Strata có F2 single-file rename, Windows native operations/undo và custom actions; chưa thấy batch dialog/pattern engine Windows. Không được viện dẫn script Linux như feature có sẵn trên Windows.

### Bằng chứng Strata ghim commit 35b24d9a
- [inline_edit.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/ui/browser/inline_edit.rs)
- [operations.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/services/operations.rs)
- [local_operations_windows.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/src/adapters/local_operations_windows.rs)
- [windows_operations.rs](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/crates/strata-native-io/src/windows_operations.rs)
- [custom-actions.md](https://github.com/IKurokami/strata/blob/35b24d9a6bb6ece1240ffbad0e5a8e91b2adb0d2/docs/custom-actions.md)

## Thiết kế Windows-native / tích hợp vào code hiện tại
BulkRenamePlan là pure function: selected FileIdentity + name/path + rule (find/replace, regex opt-in, numbering, prefix/suffix, case, extension preserve). Preview 2-column old/new, mark unchanged/invalid/conflict. Validate Windows reserved device names, trailing dots/spaces, case-insensitive duplicate, in-batch swaps, long paths, source changed, current-directory-only in first slice. Execute per-volume using GUID temp names, no-clobber publish, replay-safe journal grouped operation, restore/rollback on partial failure. Use adapter in strata-native-io; never shell rename.

## Chia lát triển khai
MVP: selection of local files in one directory, sequential numbering and find/replace preview, preflight all conflicts, execute and Ctrl+Z entire confirmed batch. Next regex/templates, move across directories only with explicit extra design, profiles/saved presets.

## Security, correctness, performance, cancel
Atomicity across N renames cannot be guaranteed; journal per-step plus compensating rollback, preserve stranded temp path and report. Case-only NTFS, open handles, network share behavior, ACL, watcher reorder, duplicate extension and Unicode canonicalization. Cancel only at safe transaction checkpoints; no silent replace.

## Tiêu chí nghiệm thu trước merge implementation
5 selected files including swaps/case-only, staged name conflict, locked source, injected failure at middle, Unicode file names and Ctrl+Z/redo produce correct outcomes; destination never overwritten; identical-name noop counted; UI lists exact partial results on rollback failure.

## Quy ước chung
Áp dụng [cross-cutting architecture](00-cross-cutting-architecture.md): GTK không làm blocking I/O; mọi request có generation và hủy; log không chứa path/secret; dùng code Windows thay cho Linux-only APIs; kiểm thử native Windows và negative cases trước khi tuyên bố shipped. Đọc [roadmap](00-roadmap.md) để tránh triển khai trùng các tính năng đã có.
